use alloc::vec;
use alloc::vec::Vec;

use league_seat_types::{
    AssetId, BundleDraftAction, ContractDeliveryOrder, CounterpartyAssumptions, HumanWhatIfRequest,
    PreviewTieMode, SeatIntent, SeatKeyCode, SeatKeyEvent, SeatObservation, Units,
};

use crate::state::{
    ALL_MARKET_KINDS, BundleComposerFields, BundleSection, CenterTab, FocusZone, MarketActionKind,
    MarketComposerFields, SeatUiState, visible_bundle_sections,
};

pub(crate) fn apply_seat_key(
    obs: &SeatObservation,
    state: &mut SeatUiState,
    key: SeatKeyEvent,
) -> Option<SeatIntent> {
    if state.chronicle_open {
        return handle_chronicle_key(obs, state, key);
    }
    if state.composer_edit_active {
        return handle_composer_text_key(obs, state, key);
    }
    if state.notes_edit_active {
        return handle_notes_text_key(state, key);
    }

    match key.code {
        SeatKeyCode::Char('c') if key.modifiers.ctrl => Some(SeatIntent::quit()),
        SeatKeyCode::Char('q') | SeatKeyCode::Esc => Some(SeatIntent::quit()),
        SeatKeyCode::Char('l') if key.modifiers.ctrl => Some(SeatIntent::lock_and_hand_off()),
        SeatKeyCode::Char('L') => Some(SeatIntent::lock_and_hand_off()),
        SeatKeyCode::Char('r') | SeatKeyCode::F(2) => {
            state.set_chronicle_open(true);
            clamp_chronicle_idx(obs, state);
            Some(SeatIntent::open_chronicle_modal())
        }
        SeatKeyCode::Char('c') => {
            state.plain_codes = !state.plain_codes;
            None
        }
        SeatKeyCode::Char('1') => {
            state.active_tab = CenterTab::MarketAndContracts;
            state.selection.focus_zone = FocusZone::CenterTabs;
            None
        }
        SeatKeyCode::Char('2') => {
            state.active_tab = CenterTab::WorkshopDealsPolitics;
            state.selection.focus_zone = FocusZone::CenterTabs;
            clamp_bundle_section_idx(obs, state);
            None
        }
        SeatKeyCode::Char('3') => {
            state.active_tab = CenterTab::WhatIfSandbox;
            state.selection.focus_zone = FocusZone::CenterTabs;
            None
        }
        SeatKeyCode::Char('4') => None,
        SeatKeyCode::Tab => {
            state.active_tab = state.active_tab.cycle(true);
            state.selection.focus_zone = FocusZone::CenterTabs;
            clamp_bundle_section_idx(obs, state);
            None
        }
        SeatKeyCode::BackTab => {
            state.active_tab = state.active_tab.cycle(false);
            state.selection.focus_zone = FocusZone::CenterTabs;
            clamp_bundle_section_idx(obs, state);
            None
        }
        SeatKeyCode::F(6) => {
            state.selection.focus_zone = state.selection.focus_zone.next();
            None
        }
        SeatKeyCode::Char('e') => {
            state.composer_edit_active = true;
            state.selection.focus_zone = FocusZone::CenterTabs;
            None
        }
        SeatKeyCode::Char('n') => {
            state.notes_edit_active = true;
            state.selection.focus_zone = FocusZone::PrivateDrawer;
            None
        }
        SeatKeyCode::Char('p') => {
            state.active_tab = CenterTab::WhatIfSandbox;
            state.selection.focus_zone = FocusZone::CenterTabs;
            Some(SeatIntent::request_what_if(build_what_if_request(
                obs, state,
            )))
        }
        _ => dispatch_focused_zone_key(obs, state, key),
    }
}

fn handle_chronicle_key(
    obs: &SeatObservation,
    state: &mut SeatUiState,
    key: SeatKeyEvent,
) -> Option<SeatIntent> {
    match key.code {
        SeatKeyCode::Char('c') if key.modifiers.ctrl => Some(SeatIntent::quit()),
        SeatKeyCode::Char('l') if key.modifiers.ctrl => {
            state.set_chronicle_open(false);
            Some(SeatIntent::lock_and_hand_off())
        }
        SeatKeyCode::Esc | SeatKeyCode::Char('q') | SeatKeyCode::Char('r') | SeatKeyCode::F(2) => {
            state.set_chronicle_open(false);
            Some(SeatIntent::close_chronicle_modal())
        }
        SeatKeyCode::Left | SeatKeyCode::Char('[') => {
            state.chronicle_receipt_idx = state.chronicle_receipt_idx.saturating_sub(1);
            state.chronicle_scroll = 0;
            None
        }
        SeatKeyCode::Right | SeatKeyCode::Char(']') => {
            let max_idx = obs.public_receipts.len().saturating_sub(1);
            state.chronicle_receipt_idx = (state.chronicle_receipt_idx + 1).min(max_idx);
            state.chronicle_scroll = 0;
            None
        }
        SeatKeyCode::Up => {
            state.chronicle_scroll = state.chronicle_scroll.saturating_sub(1);
            None
        }
        SeatKeyCode::Down => {
            state.chronicle_scroll = state.chronicle_scroll.saturating_add(1);
            None
        }
        SeatKeyCode::PageUp => {
            state.chronicle_scroll = state.chronicle_scroll.saturating_sub(6);
            None
        }
        SeatKeyCode::PageDown => {
            state.chronicle_scroll = state.chronicle_scroll.saturating_add(6);
            None
        }
        SeatKeyCode::Home => {
            state.chronicle_scroll = 0;
            None
        }
        SeatKeyCode::Char('c') => {
            state.plain_codes = !state.plain_codes;
            None
        }
        _ => None,
    }
}

fn handle_composer_text_key(
    obs: &SeatObservation,
    state: &mut SeatUiState,
    key: SeatKeyEvent,
) -> Option<SeatIntent> {
    match key.code {
        SeatKeyCode::Char('c') if key.modifiers.ctrl => Some(SeatIntent::quit()),
        SeatKeyCode::Esc => {
            state.composer_edit_active = false;
            None
        }
        SeatKeyCode::Enter => {
            apply_composer_draft_overrides(obs, state);
            state.composer_edit_active = false;
            None
        }
        SeatKeyCode::Backspace => {
            state.composer_draft.pop();
            None
        }
        SeatKeyCode::Char(ch) if !key.modifiers.ctrl => {
            if !ch.is_control() && state.composer_draft.chars().count() < 120 {
                state.composer_draft.push(ch);
            }
            None
        }
        _ => None,
    }
}

fn handle_notes_text_key(state: &mut SeatUiState, key: SeatKeyEvent) -> Option<SeatIntent> {
    match key.code {
        SeatKeyCode::Char('c') if key.modifiers.ctrl => Some(SeatIntent::quit()),
        SeatKeyCode::Esc | SeatKeyCode::Enter => {
            state.notes_edit_active = false;
            None
        }
        SeatKeyCode::Backspace => {
            state.notes_draft.pop();
            None
        }
        SeatKeyCode::Char(ch) if !key.modifiers.ctrl => {
            if !ch.is_control() && state.notes_draft.chars().count() < 240 {
                state.notes_draft.push(ch);
            }
            None
        }
        _ => None,
    }
}

fn dispatch_focused_zone_key(
    obs: &SeatObservation,
    state: &mut SeatUiState,
    key: SeatKeyEvent,
) -> Option<SeatIntent> {
    match state.selection.focus_zone {
        FocusZone::PublicLedger => {
            match key.code {
                SeatKeyCode::Up => {
                    state.public_scroll = state.public_scroll.saturating_sub(1);
                    state.selection.ledger_row_idx =
                        state.selection.ledger_row_idx.saturating_sub(1);
                }
                SeatKeyCode::Down => {
                    state.public_scroll = state.public_scroll.saturating_add(1);
                    state.selection.ledger_row_idx =
                        state.selection.ledger_row_idx.saturating_add(1);
                }
                SeatKeyCode::PageUp => {
                    state.public_scroll = state.public_scroll.saturating_sub(6);
                }
                SeatKeyCode::PageDown => {
                    state.public_scroll = state.public_scroll.saturating_add(6);
                }
                SeatKeyCode::Home => {
                    state.public_scroll = 0;
                    state.selection.ledger_row_idx = 0;
                }
                _ => {}
            }
            None
        }
        FocusZone::PrivateDrawer => {
            match key.code {
                SeatKeyCode::Up => {
                    state.drawer_scroll = state.drawer_scroll.saturating_sub(1);
                    state.selection.drawer_row_idx =
                        state.selection.drawer_row_idx.saturating_sub(1);
                }
                SeatKeyCode::Down => {
                    state.drawer_scroll = state.drawer_scroll.saturating_add(1);
                    state.selection.drawer_row_idx =
                        state.selection.drawer_row_idx.saturating_add(1);
                }
                SeatKeyCode::PageUp => {
                    state.drawer_scroll = state.drawer_scroll.saturating_sub(6);
                }
                SeatKeyCode::PageDown => {
                    state.drawer_scroll = state.drawer_scroll.saturating_add(6);
                }
                SeatKeyCode::Home => {
                    state.drawer_scroll = 0;
                    state.selection.drawer_row_idx = 0;
                }
                _ => {}
            }
            None
        }
        FocusZone::CenterTabs => match state.active_tab {
            CenterTab::MarketAndContracts => handle_market_tab_key(obs, state, key),
            CenterTab::WorkshopDealsPolitics => handle_bundle_tab_key(obs, state, key),
            CenterTab::WhatIfSandbox => handle_what_if_tab_key(obs, state, key),
        },
    }
}

fn handle_market_tab_key(
    obs: &SeatObservation,
    state: &mut SeatUiState,
    key: SeatKeyEvent,
) -> Option<SeatIntent> {
    let kind_len = ALL_MARKET_KINDS.len();
    let kind = ALL_MARKET_KINDS[state.selection.market_kind_idx % kind_len];
    let field_count = market_field_count(kind);
    let n = obs.spec.seat_count.max(1);

    match key.code {
        SeatKeyCode::Char('[') => {
            state.selection.market_kind_idx =
                (state.selection.market_kind_idx + kind_len - 1) % kind_len;
            state.selection.market_field_idx = 0;
            state.center_scroll = 0;
            None
        }
        SeatKeyCode::Char(']') => {
            state.selection.market_kind_idx = (state.selection.market_kind_idx + 1) % kind_len;
            state.selection.market_field_idx = 0;
            state.center_scroll = 0;
            None
        }
        SeatKeyCode::Up => {
            if state.selection.market_field_idx == 0 {
                state.center_scroll = state.center_scroll.saturating_sub(1);
            } else {
                state.selection.market_field_idx -= 1;
            }
            None
        }
        SeatKeyCode::Down => {
            if state.selection.market_field_idx + 1 < field_count {
                state.selection.market_field_idx += 1;
            } else {
                state.center_scroll = state.center_scroll.saturating_add(1);
            }
            None
        }
        SeatKeyCode::PageUp => {
            state.center_scroll = state.center_scroll.saturating_sub(6);
            None
        }
        SeatKeyCode::PageDown => {
            state.center_scroll = state.center_scroll.saturating_add(6);
            None
        }
        SeatKeyCode::Home => {
            state.selection.market_field_idx = 0;
            state.center_scroll = 0;
            None
        }
        SeatKeyCode::Left | SeatKeyCode::Char('-') => {
            adjust_market_field(
                &mut state.market_fields,
                kind,
                state.selection.market_field_idx,
                -1,
                n,
            );
            None
        }
        SeatKeyCode::Right
        | SeatKeyCode::Char('+')
        | SeatKeyCode::Char('=')
        | SeatKeyCode::Char(' ') => {
            adjust_market_field(
                &mut state.market_fields,
                kind,
                state.selection.market_field_idx,
                1,
                n,
            );
            None
        }
        SeatKeyCode::Enter => {
            if obs.cancel_locked && kind == MarketActionKind::CancelOffer {
                return None;
            }
            Some(SeatIntent::submit_market_action(
                state.market_fields.build_action(obs, kind),
            ))
        }
        _ => None,
    }
}

fn handle_bundle_tab_key(
    obs: &SeatObservation,
    state: &mut SeatUiState,
    key: SeatKeyEvent,
) -> Option<SeatIntent> {
    let sections = visible_bundle_sections(obs.turn);
    let sec_len = sections.len().max(1);
    let sec_idx = state.selection.bundle_section_idx % sec_len;
    state.selection.bundle_section_idx = sec_idx;
    let section = sections[sec_idx];
    let field_count = bundle_field_count(section);
    let n = obs.spec.seat_count.max(1);

    match key.code {
        SeatKeyCode::Char('[') => {
            state.selection.bundle_section_idx = (sec_idx + sec_len - 1) % sec_len;
            state.selection.bundle_field_idx = 0;
            state.center_scroll = 0;
            None
        }
        SeatKeyCode::Char(']') => {
            state.selection.bundle_section_idx = (sec_idx + 1) % sec_len;
            state.selection.bundle_field_idx = 0;
            state.center_scroll = 0;
            None
        }
        SeatKeyCode::Up => {
            if state.selection.bundle_field_idx == 0 {
                state.center_scroll = state.center_scroll.saturating_sub(1);
            } else {
                state.selection.bundle_field_idx -= 1;
            }
            None
        }
        SeatKeyCode::Down => {
            if state.selection.bundle_field_idx + 1 < field_count {
                state.selection.bundle_field_idx += 1;
            } else {
                state.center_scroll = state.center_scroll.saturating_add(1);
            }
            None
        }
        SeatKeyCode::PageUp => {
            state.center_scroll = state.center_scroll.saturating_sub(6);
            None
        }
        SeatKeyCode::PageDown => {
            state.center_scroll = state.center_scroll.saturating_add(6);
            None
        }
        SeatKeyCode::Home => {
            state.selection.bundle_field_idx = 0;
            state.center_scroll = 0;
            None
        }
        SeatKeyCode::Left | SeatKeyCode::Char('-') => {
            adjust_bundle_field(
                &mut state.bundle_fields,
                section,
                state.selection.bundle_field_idx,
                -1,
                n,
                obs.spec
                    .seats
                    .get(usize::from(obs.seat))
                    .map_or(1, |c| c.converters.len()),
                obs.spec.projects.len(),
            );
            None
        }
        SeatKeyCode::Right
        | SeatKeyCode::Char('+')
        | SeatKeyCode::Char('=')
        | SeatKeyCode::Char(' ') => {
            adjust_bundle_field(
                &mut state.bundle_fields,
                section,
                state.selection.bundle_field_idx,
                1,
                n,
                obs.spec
                    .seats
                    .get(usize::from(obs.seat))
                    .map_or(1, |c| c.converters.len()),
                obs.spec.projects.len(),
            );
            None
        }
        SeatKeyCode::Enter => Some(SeatIntent::update_bundle(
            state.bundle_fields.build_stage_op(obs, section),
        )),
        SeatKeyCode::Delete | SeatKeyCode::Backspace => Some(SeatIntent::update_bundle(
            state.bundle_fields.build_clear_op(section),
        )),
        SeatKeyCode::Char('X') => Some(SeatIntent::update_bundle(BundleDraftAction::ClearAll)),
        _ => None,
    }
}

fn handle_what_if_tab_key(
    obs: &SeatObservation,
    state: &mut SeatUiState,
    key: SeatKeyEvent,
) -> Option<SeatIntent> {
    let offer_ids: Vec<u64> = obs
        .offers
        .values()
        .filter(|o| o.maker != obs.seat)
        .map(|o| o.offer_id)
        .collect();
    let check_ids: Vec<u64> = obs.checks.keys().copied().collect();
    let row_total = 5 + offer_ids.len() + check_ids.len();

    match key.code {
        SeatKeyCode::Up => {
            state.selection.what_if_row_idx = state.selection.what_if_row_idx.saturating_sub(1);
            if state.selection.what_if_row_idx == 0 {
                state.center_scroll = state.center_scroll.saturating_sub(1);
            }
            None
        }
        SeatKeyCode::Down => {
            if state.selection.what_if_row_idx + 1 < row_total {
                state.selection.what_if_row_idx += 1;
            } else {
                state.center_scroll = state.center_scroll.saturating_add(1);
            }
            None
        }
        SeatKeyCode::PageUp => {
            state.center_scroll = state.center_scroll.saturating_sub(6);
            None
        }
        SeatKeyCode::PageDown => {
            state.center_scroll = state.center_scroll.saturating_add(6);
            None
        }
        SeatKeyCode::Home => {
            state.selection.what_if_row_idx = 0;
            state.center_scroll = 0;
            None
        }
        SeatKeyCode::Left
        | SeatKeyCode::Right
        | SeatKeyCode::Char(' ')
        | SeatKeyCode::Char('+')
        | SeatKeyCode::Char('-')
        | SeatKeyCode::Char('=') => {
            toggle_what_if_row(obs, state, &offer_ids, &check_ids);
            None
        }
        SeatKeyCode::Enter => Some(SeatIntent::request_what_if(build_what_if_request(
            obs, state,
        ))),
        _ => None,
    }
}

fn toggle_what_if_row(
    obs: &SeatObservation,
    state: &mut SeatUiState,
    offer_ids: &[u64],
    check_ids: &[u64],
) {
    let row = state.selection.what_if_row_idx;
    match row {
        0 => {
            state.tie_mode = match state.tie_mode {
                PreviewTieMode::NeutralSeed => PreviewTieMode::PessimisticForSeat,
                PreviewTieMode::PessimisticForSeat => PreviewTieMode::NeutralSeed,
            };
        }
        1 => state.include_draft_market_action = !state.include_draft_market_action,
        2 => state.assume_cosign_swaps = !state.assume_cosign_swaps,
        3 => state.assume_cosign_dual_escrows = !state.assume_cosign_dual_escrows,
        4 => state.assume_peer_deliveries = !state.assume_peer_deliveries,
        _ => {
            let offset = row - 5;
            if offset < offer_ids.len() {
                if !obs.cancel_locked {
                    toggle_u64(&mut state.assumed_cancelled_offer_ids, offer_ids[offset]);
                }
            } else {
                let cidx = offset - offer_ids.len();
                if let Some(&cid) = check_ids.get(cidx) {
                    toggle_u64(&mut state.assumed_bounced_check_ids, cid);
                }
            }
        }
    }
}

fn toggle_u64(items: &mut Vec<u64>, target: u64) {
    if let Some(pos) = items.iter().position(|&id| id == target) {
        items.remove(pos);
    } else {
        items.push(target);
    }
}

pub(crate) fn build_what_if_request(
    obs: &SeatObservation,
    state: &SeatUiState,
) -> HumanWhatIfRequest {
    let kind = ALL_MARKET_KINDS[state.selection.market_kind_idx % ALL_MARKET_KINDS.len()];
    let market_actions = if state.include_draft_market_action {
        vec![state.market_fields.build_action(obs, kind)]
    } else {
        Vec::new()
    };
    let mut assumptions = CounterpartyAssumptions::default();
    if !obs.cancel_locked {
        assumptions.assumed_cancelled_offer_ids = state.assumed_cancelled_offer_ids.clone();
    }
    if state.assume_cosign_swaps {
        assumptions.assumed_cosigned_swaps = state.staged_bundle.atomic_swaps.clone();
    }
    if state.assume_cosign_dual_escrows {
        assumptions.assumed_cosigned_dual_escrows = state.staged_bundle.dual_escrows.clone();
    }
    if state.assume_peer_deliveries {
        for dual in obs.dual_escrows.values() {
            if dual.buyer == obs.seat {
                assumptions.assumed_peer_deliveries.push((
                    dual.seller,
                    ContractDeliveryOrder {
                        counterparty: dual.buyer,
                        good: dual.good,
                        gross_amount: dual.required_units,
                        domain_id: dual.domain_id,
                        dual_escrow_id: Some(dual.escrow_id),
                    },
                ));
            }
        }
    }
    assumptions.assumed_bounced_check_ids = state.assumed_bounced_check_ids.clone();

    HumanWhatIfRequest {
        seat: obs.seat,
        market_actions,
        cancel_lock_at: None,
        bundle: state.staged_bundle.clone(),
        tie_mode: state.tie_mode,
        assumptions,
    }
}

pub(crate) fn market_field_count(kind: MarketActionKind) -> usize {
    match kind {
        MarketActionKind::PostOffer => 7,
        MarketActionKind::CancelOffer
        | MarketActionKind::CancelCheck
        | MarketActionKind::CancelExpiredEscrow => 1,
        MarketActionKind::CreateCheck => 6,
        MarketActionKind::CreateEscrow => 7,
        MarketActionKind::RecordIntentPledge => 5,
        MarketActionKind::StandForOffice => 5,
        MarketActionKind::FlagWashFreeze => 1,
        MarketActionKind::TownImport => 3,
    }
}

pub(crate) fn bundle_field_count(section: BundleSection) -> usize {
    match section {
        BundleSection::ConverterRuns | BundleSection::ProjectDeliveries => 2,
        BundleSection::AtomicSwaps => 7,
        BundleSection::ContractDeliveries => 5,
        BundleSection::DualEscrows => 9,
        BundleSection::DebtRepayment | BundleSection::CheckCashes => 1,
        BundleSection::EscrowFinishes => 3,
        BundleSection::CouncilVote | BundleSection::CharterBids => 2,
    }
}

fn step_units(val: Units, delta: i8, step: Units, min_val: Units) -> Units {
    if delta >= 0 {
        val.saturating_add(step)
    } else {
        val.saturating_sub(step).max(min_val)
    }
}

fn adjust_opt_u8(val: &mut Option<u8>, delta: i8, upper: u8) {
    let cap = upper.max(1);
    *val = match (*val, delta >= 0) {
        (None, true) => Some(0),
        (Some(d), true) if d + 1 < cap => Some(d + 1),
        (Some(_), true) => None,
        (None, false) => Some(cap - 1),
        (Some(0), false) => None,
        (Some(d), false) => Some(d - 1),
    };
}

fn adjust_market_field(
    fields: &mut MarketComposerFields,
    kind: MarketActionKind,
    field_idx: usize,
    delta: i8,
    seat_count: u8,
) {
    let n = seat_count.max(1);
    let step_idx = |curr: u8, modulo: u8| -> u8 {
        let m = modulo.max(1);
        if delta >= 0 {
            (curr + 1) % m
        } else {
            (curr + m - 1) % m
        }
    };

    match kind {
        MarketActionKind::PostOffer => match field_idx {
            0 => fields.side_buy = !fields.side_buy,
            1 => fields.good_kind_raw = !fields.good_kind_raw,
            2 => fields.good_idx = step_idx(fields.good_idx, n),
            3 => fields.amount = step_units(fields.amount, delta, 250, 100),
            4 => {
                fields.price_per_unit_cred =
                    step_units(fields.price_per_unit_cred, delta, 100, 100);
            }
            5 => adjust_opt_u8(&mut fields.domain_id, delta, 4),
            6 => fields.is_hybrid = !fields.is_hybrid,
            _ => {}
        },
        MarketActionKind::CancelOffer
        | MarketActionKind::CancelCheck
        | MarketActionKind::CancelExpiredEscrow => {
            if delta >= 0 {
                fields.target_id = fields.target_id.saturating_add(1);
            } else {
                fields.target_id = fields.target_id.saturating_sub(1).max(1);
            }
        }
        MarketActionKind::CreateCheck => match field_idx {
            0 => fields.counterparty = step_idx(fields.counterparty, n),
            1 => fields.check_asset_mode = step_idx(fields.check_asset_mode, 3),
            2 => fields.good_idx = step_idx(fields.good_idx, n),
            3 => fields.amount = step_units(fields.amount, delta, 250, 100),
            4 => fields.deliver_min = step_units(fields.deliver_min, delta, 250, 100),
            5 => {
                if delta >= 0 {
                    fields.expiration_turn = (fields.expiration_turn + 1).min(32);
                } else {
                    fields.expiration_turn = fields.expiration_turn.saturating_sub(1);
                }
            }
            _ => {}
        },
        MarketActionKind::CreateEscrow => match field_idx {
            0 => fields.counterparty = step_idx(fields.counterparty, n),
            1 => fields.amount = step_units(fields.amount, delta, 250, 100),
            2 => fields.has_condition = !fields.has_condition,
            3 => fields.has_bond = !fields.has_bond,
            4 => fields.bond_buyer_dvp = !fields.bond_buyer_dvp,
            5 => {
                fields.bond_required_units =
                    step_units(fields.bond_required_units, delta, 250, 100);
            }
            6 => {
                if delta >= 0 {
                    fields.cancel_after_turn = (fields.cancel_after_turn + 1).min(32);
                } else {
                    fields.cancel_after_turn = fields.cancel_after_turn.saturating_sub(1);
                }
            }
            _ => {}
        },
        MarketActionKind::RecordIntentPledge => match field_idx {
            0 => {
                fields.target_id = if delta >= 0 {
                    fields.target_id.saturating_add(1)
                } else {
                    fields.target_id.saturating_sub(1).max(1)
                }
            }
            1 => fields.counterparty = step_idx(fields.counterparty, n),
            2 => fields.pledge_target_kind = step_idx(fields.pledge_target_kind, 3),
            3 => {
                if delta >= 0 {
                    fields.office_turn = (fields.office_turn + 4).clamp(4, 32);
                } else {
                    fields.office_turn = fields.office_turn.saturating_sub(4).max(4);
                }
            }
            4 => fields.honor_weight = step_units(fields.honor_weight, delta, 100, 100),
            _ => {}
        },
        MarketActionKind::StandForOffice => match field_idx {
            0 => {
                fields.platform_raw_tariff_bps = adjust_bps(fields.platform_raw_tariff_bps, delta);
            }
            1 => {
                fields.platform_refined_tariff_bps =
                    adjust_bps(fields.platform_refined_tariff_bps, delta);
            }
            2 => adjust_opt_u8(&mut fields.platform_admit_domain, delta, 4),
            3 => adjust_opt_u8(&mut fields.platform_revoke_domain, delta, 4),
            4 => fields.counterparty = step_idx(fields.counterparty, n),
            _ => {}
        },
        MarketActionKind::FlagWashFreeze => {
            fields.counterparty = step_idx(fields.counterparty, n);
        }
        MarketActionKind::TownImport => match field_idx {
            0 => fields.good_kind_raw = !fields.good_kind_raw,
            1 => fields.good_idx = step_idx(fields.good_idx, n),
            2 => fields.amount = step_units(fields.amount, delta, 250, 100),
            _ => {}
        },
    }
}

fn adjust_bps(val: u16, delta: i8) -> u16 {
    if delta >= 0 {
        val.saturating_add(100).min(3_500)
    } else {
        val.saturating_sub(100)
    }
}

fn adjust_bundle_field(
    fields: &mut BundleComposerFields,
    section: BundleSection,
    field_idx: usize,
    delta: i8,
    seat_count: u8,
    recipe_count: usize,
    project_count: usize,
) {
    let n = seat_count.max(1);
    let step_idx = |curr: u8, modulo: u8| -> u8 {
        let m = modulo.max(1);
        if delta >= 0 {
            (curr + 1) % m
        } else {
            (curr + m - 1) % m
        }
    };

    match section {
        BundleSection::ConverterRuns => match field_idx {
            0 => {
                let m = recipe_count.max(1);
                fields.recipe_idx = if delta >= 0 {
                    (fields.recipe_idx + 1) % m
                } else {
                    (fields.recipe_idx + m - 1) % m
                };
            }
            1 => {
                fields.runs = if delta >= 0 {
                    fields.runs.saturating_add(1).min(16)
                } else {
                    fields.runs.saturating_sub(1)
                };
            }
            _ => {}
        },
        BundleSection::ProjectDeliveries => match field_idx {
            0 => {
                let m = u16::try_from(project_count.max(1)).unwrap_or(1);
                let zero_based = fields.project_id.saturating_sub(1) % m;
                let next = if delta >= 0 {
                    (zero_based + 1) % m
                } else {
                    (zero_based + m - 1) % m
                };
                fields.project_id = next + 1;
            }
            1 => {
                fields.batches = if delta >= 0 {
                    fields.batches.saturating_add(1).min(16)
                } else {
                    fields.batches.saturating_sub(1)
                };
            }
            _ => {}
        },
        BundleSection::AtomicSwaps => match field_idx {
            0 => {
                fields.swap_id = if delta >= 0 {
                    fields.swap_id.saturating_add(1)
                } else {
                    fields.swap_id.saturating_sub(1).max(1)
                }
            }
            1 => fields.swap_taker = step_idx(fields.swap_taker, n),
            2 => fields.swap_maker_raw = !fields.swap_maker_raw,
            3 => fields.swap_maker_good = step_idx(fields.swap_maker_good, n),
            4 => fields.swap_maker_amount = step_units(fields.swap_maker_amount, delta, 250, 100),
            5 => fields.swap_taker_good = step_idx(fields.swap_taker_good, n),
            6 => fields.swap_taker_amount = step_units(fields.swap_taker_amount, delta, 250, 100),
            _ => {}
        },
        BundleSection::ContractDeliveries => match field_idx {
            0 => fields.cd_counterparty = step_idx(fields.cd_counterparty, n),
            1 => fields.cd_raw = !fields.cd_raw,
            2 => fields.cd_good = step_idx(fields.cd_good, n),
            3 => fields.cd_amount = step_units(fields.cd_amount, delta, 250, 100),
            4 => {
                fields.cd_dual_escrow_id = match (fields.cd_dual_escrow_id, delta >= 0) {
                    (None, true) => Some(1),
                    (Some(v), true) => Some(v.saturating_add(1)),
                    (None, false) => None,
                    (Some(v), false) if v <= 1 => None,
                    (Some(v), false) => Some(v - 1),
                };
            }
            _ => {}
        },
        BundleSection::DualEscrows => match field_idx {
            0 => {
                fields.dual_id = if delta >= 0 {
                    fields.dual_id.saturating_add(1)
                } else {
                    fields.dual_id.saturating_sub(1).max(1)
                }
            }
            1 => fields.dual_seller = step_idx(fields.dual_seller, n),
            2 => fields.dual_raw = !fields.dual_raw,
            3 => fields.dual_good = step_idx(fields.dual_good, n),
            4 => {
                fields.dual_required_units = step_units(fields.dual_required_units, delta, 250, 100)
            }
            5 => fields.dual_payment_cred = step_units(fields.dual_payment_cred, delta, 250, 200),
            6 => {
                fields.dual_buyer_bond_cred =
                    step_units(fields.dual_buyer_bond_cred, delta, 100, 100)
            }
            7 => {
                fields.dual_seller_bond_cred =
                    step_units(fields.dual_seller_bond_cred, delta, 100, 100)
            }
            8 => {
                if delta >= 0 {
                    fields.dual_deadline_turn = (fields.dual_deadline_turn + 1).min(32);
                } else {
                    fields.dual_deadline_turn = fields.dual_deadline_turn.saturating_sub(1);
                }
            }
            _ => {}
        },
        BundleSection::DebtRepayment => {
            fields.debt_repayment_cred = step_units(fields.debt_repayment_cred, delta, 250, 0);
        }
        BundleSection::EscrowFinishes => match field_idx {
            0 => {
                fields.escrow_finish_id = if delta >= 0 {
                    fields.escrow_finish_id.saturating_add(1)
                } else {
                    fields.escrow_finish_id.saturating_sub(1).max(1)
                }
            }
            1 => fields.escrow_use_preimage = !fields.escrow_use_preimage,
            2 => {
                fields.escrow_preimage_byte = if delta >= 0 {
                    fields.escrow_preimage_byte.wrapping_add(1).max(1)
                } else {
                    fields.escrow_preimage_byte.saturating_sub(1).max(1)
                };
            }
            _ => {}
        },
        BundleSection::CheckCashes => {
            fields.check_cash_id = if delta >= 0 {
                fields.check_cash_id.saturating_add(1)
            } else {
                fields.check_cash_id.saturating_sub(1).max(1)
            };
        }
        BundleSection::CouncilVote => match field_idx {
            0 => fields.vote_tm = step_idx(fields.vote_tm, n),
            1 => fields.vote_cc = step_idx(fields.vote_cc, n),
            _ => {}
        },
        BundleSection::CharterBids => match field_idx {
            0 => fields.charter_role_idx = step_idx(fields.charter_role_idx, 3),
            1 => fields.charter_bid_cred = step_units(fields.charter_bid_cred, delta, 100, 0),
            _ => {}
        },
    }
}

fn apply_composer_draft_overrides(obs: &SeatObservation, state: &mut SeatUiState) {
    let n = obs.spec.seat_count.max(1);
    for token in state.composer_draft.split_whitespace() {
        let Some((k, v)) = token.split_once('=') else {
            continue;
        };
        match k {
            "amt" => {
                if let Ok(parsed) = v.parse::<Units>() {
                    state.market_fields.amount = parsed.max(1);
                    state.bundle_fields.cd_amount = parsed.max(1);
                    state.bundle_fields.dual_required_units = parsed.max(1);
                }
            }
            "px" => {
                if let Ok(parsed) = v.parse::<Units>() {
                    state.market_fields.price_per_unit_cred = parsed.max(1);
                    state.bundle_fields.dual_payment_cred = parsed.max(2);
                }
            }
            "id" => {
                if let Ok(parsed) = v.parse::<u64>() {
                    state.market_fields.target_id = parsed.max(1);
                    state.bundle_fields.check_cash_id = parsed.max(1);
                    state.bundle_fields.escrow_finish_id = parsed.max(1);
                    state.bundle_fields.swap_id = parsed.max(1);
                    state.bundle_fields.dual_id = parsed.max(1);
                }
            }
            "peer" => {
                if let Ok(parsed) = v.parse::<u8>() {
                    state.market_fields.counterparty = parsed % n;
                    state.bundle_fields.cd_counterparty = parsed % n;
                    state.bundle_fields.swap_taker = parsed % n;
                    state.bundle_fields.dual_seller = parsed % n;
                }
            }
            "runs" => {
                if let Ok(parsed) = v.parse::<u32>() {
                    state.bundle_fields.runs = parsed;
                }
            }
            "batches" => {
                if let Ok(parsed) = v.parse::<u32>() {
                    state.bundle_fields.batches = parsed;
                }
            }
            "debt" => {
                if let Ok(parsed) = v.parse::<Units>() {
                    state.bundle_fields.debt_repayment_cred = parsed.max(0);
                }
            }
            "bid" => {
                if let Ok(parsed) = v.parse::<Units>() {
                    state.bundle_fields.charter_bid_cred = parsed.max(0);
                }
            }
            _ => {}
        }
    }
}

fn clamp_bundle_section_idx(obs: &SeatObservation, state: &mut SeatUiState) {
    let len = visible_bundle_sections(obs.turn).len();
    if len > 0 {
        state.selection.bundle_section_idx %= len;
    } else {
        state.selection.bundle_section_idx = 0;
    }
}

fn clamp_chronicle_idx(obs: &SeatObservation, state: &mut SeatUiState) {
    if obs.public_receipts.is_empty() {
        state.chronicle_receipt_idx = 0;
    } else {
        state.chronicle_receipt_idx = obs.public_receipts.len() - 1;
    }
    state.chronicle_scroll = 0;
}

pub(crate) fn preview_selected_asset(fields: &MarketComposerFields, seat_count: u8) -> AssetId {
    fields.selected_good(seat_count)
}
