//! Trusted Club Mode host session wrapper in the `league-tui` binary crate.
//!
//! Owns [`ClubHost`], save/seal persistence, and `EvalVrf` sweeps, builds one per-seat
//! [`SeatObservation`] by value carrying strictly that seat's `own_mandate`, and renders via
//! `league-seat-ui` without placing any seat logic in `league-tui`.

use std::collections::BTreeMap;
use std::path::Path;

use league_core::{
    ClubHost, EngineError, OrderReveal, PublicSweepReceipt, Seat, SeatObservation, TableId,
    TurnPhase,
};
use league_gen::{GenError, RingTable, ring_table_to_table_spec};
use league_seat_types::CommodityDisplayMap;
use ratatui::Frame;

/// Host session bridging a `ClubHost` table to seat-isolated `league-seat-ui` views.
#[derive(Debug)]
#[allow(dead_code)]
pub struct TuiHostSession {
    host: ClubHost,
    commodities: Option<CommodityDisplayMap>,
}

#[allow(dead_code)]
impl TuiHostSession {
    /// Bootstraps a Club Mode session on the canonical `TableSpec::standard_club_chiral_ring_with_setup`.
    pub fn from_standard_setup(
        table_id: TableId,
        seat_count: u8,
        layout_seed: u64,
        bot_tie_seed: u64,
    ) -> Result<Self, EngineError> {
        let host = ClubHost::from_standard_setup(table_id, seat_count, layout_seed, bot_tie_seed)?;
        Ok(Self {
            host,
            commodities: None,
        })
    }

    /// Promotes a feasibility-checked `SeedPool::Dev` [`RingTable`] into a live Club Mode session
    /// with its derived [`CommodityDisplayMap`].
    pub fn from_ring_table(
        table: &RingTable,
        table_id: TableId,
        human_seats: &[u8],
        bot_tie_seed: u64,
    ) -> Result<Self, GenError> {
        let (spec, cmap, deal, eval_seed) = ring_table_to_table_spec(table, table_id, human_seats)?;
        let host = ClubHost::new(spec, table.seed, deal, eval_seed, bot_tie_seed)
            .map_err(|err| GenError::InvalidConfig(err.to_string()))?;
        Ok(Self {
            host,
            commodities: Some(cmap.to_display_map()),
        })
    }

    /// Returns the number of seats at the table (`4..=6`).
    #[must_use]
    pub const fn seat_count(&self) -> u8 {
        self.host.seat_count()
    }

    /// Returns the active turn number (`1..=32`).
    #[must_use]
    pub const fn turn(&self) -> u8 {
        self.host.turn()
    }

    /// Returns the current [`TurnPhase`].
    #[must_use]
    pub const fn phase(&self) -> TurnPhase {
        self.host.phase()
    }

    /// Returns the seat-safe 30-field live [`PublicSweepReceipt`] history.
    #[must_use]
    pub fn public_receipts(&self) -> &[PublicSweepReceipt] {
        self.host.public_receipts()
    }

    /// Builds a single [`SeatObservation`] **by value** for `seat`, carrying strictly `seat`'s
    /// own dealt `PrivateMandate` and never rival seats' mandates or hidden engine state.
    pub fn build_seat_observation(&self, seat: Seat) -> Result<SeatObservation, EngineError> {
        self.host.observe_seat(seat)
    }

    /// Builds one [`SeatObservation`] **by value** per seat `0..seat_count` in ascending order,
    /// each carrying strictly that seat's own `PrivateMandate`.
    pub fn build_all_seat_observations(&self) -> Result<Vec<SeatObservation>, EngineError> {
        let count = self.seat_count();
        let mut out = Vec::with_capacity(usize::from(count));
        for s in 0..count {
            out.push(self.build_seat_observation(Seat(s))?);
        }
        Ok(out)
    }

    /// Runs `TalkAndMarket -> Commit -> Reveal -> sweep_turn` atomically on the host.
    pub fn commit_reveal_and_sweep(
        &mut self,
        reveals: &BTreeMap<u8, OrderReveal>,
    ) -> Result<PublicSweepReceipt, EngineError> {
        self.host.commit_and_reveal_all(reveals)?;
        self.host.sweep_turn()
    }

    /// Atomically persists `.save.json` and `.seal` sidecar files on the host.
    pub fn write_save_and_seal(
        &self,
        save_path: &Path,
        seal_path: &Path,
    ) -> Result<(), EngineError> {
        self.host.write_save_and_seal(save_path, seal_path)
    }

    /// Reloads a persisted Club Mode host session from `.save.json` and `.seal`.
    pub fn read_save_and_seal(
        save_path: &Path,
        seal_path: &Path,
        commodities: Option<CommodityDisplayMap>,
    ) -> Result<Self, EngineError> {
        let host = ClubHost::read_save_and_seal(save_path, seal_path)?;
        Ok(Self { host, commodities })
    }

    /// Wipes a finished `.seal` sidecar cleanly.
    pub fn wipe_seal_file(seal_path: &Path) -> Result<(), EngineError> {
        ClubHost::wipe_seal_file(seal_path)
    }

    /// Builds one [`SeatObservation`] **by value** for `seat` and delegates rendering solely to
    /// `league-seat-ui`.
    pub fn render_seat_frame(
        &self,
        frame: &mut Frame<'_>,
        seat: Seat,
        plain_codes: bool,
    ) -> Result<(), EngineError> {
        let obs = self.build_seat_observation(seat)?;
        match &self.commodities {
            Some(map) => {
                league_seat_ui::render_seat_with_commodities(frame, &obs, map, plain_codes)
            }
            None => league_seat_ui::render_seat(frame, &obs),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use league_gen::{
        GeneratorConfig, SeatCount, SeedPool, TablePurpose, TableSetupOptions,
        generate_ring_table_for_setup, is_reserved_test_seed, seed_pool,
    };
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn dev_ring_table(start_seed: u64) -> RingTable {
        let cfg = GeneratorConfig {
            seat_count: Some(SeatCount::Four),
            ..GeneratorConfig::default()
        };
        let opts = TableSetupOptions {
            purpose: TablePurpose::Dev,
            training_logged: false,
        };
        for seed in start_seed..1_000 {
            if is_reserved_test_seed(seed) || seed_pool(seed) != SeedPool::Dev {
                continue;
            }
            if let Ok((checked, _, _)) = generate_ring_table_for_setup(seed, opts, &cfg)
                && let Some(table) = checked.into_clear_table()
            {
                return table;
            }
        }
        panic!("no feasible dev table found from seed {start_seed}");
    }

    fn screen_text(terminal: &Terminal<TestBackend>) -> String {
        let buffer = terminal.backend().buffer();
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn must_ok<T, E: std::fmt::Display>(res: Result<T, E>, ctx: &str) -> T {
        match res {
            Ok(v) => v,
            Err(err) => panic!("{ctx}: {err}"),
        }
    }

    fn must_some<T>(opt: Option<T>, ctx: &str) -> T {
        match opt {
            Some(v) => v,
            None => panic!("expected Some for {ctx}"),
        }
    }

    #[test]
    fn host_session_isolates_per_seat_observations_sweeps_persists_and_renders() {
        let mut session = must_ok(
            TuiHostSession::from_standard_setup(TableId(81_001), 4, 1, 7),
            "standard club setup",
        );
        assert_eq!(session.seat_count(), 4);
        assert_eq!(session.turn(), 1);
        assert_eq!(session.phase(), TurnPhase::TalkAndMarket);
        assert!(session.public_receipts().is_empty());

        let all_obs = must_ok(
            session.build_all_seat_observations(),
            "all seat observations",
        );
        assert_eq!(all_obs.len(), 4);
        for (s, obs) in all_obs.iter().enumerate() {
            assert_eq!(usize::from(obs.seat), s);
            let own = must_some(obs.own_mandate(), "own_mandate attached");
            let own_dbg = format!("{own:?}");
            let obs_dbg = format!("{obs:?}");
            assert!(obs_dbg.contains(&own_dbg));
            for (r, rival_obs) in all_obs.iter().enumerate() {
                if r != s {
                    let rival_mandate = must_some(rival_obs.own_mandate(), "rival mandate");
                    if rival_mandate != own {
                        let rival_dbg = format!("{rival_mandate:?}");
                        assert!(
                            !obs_dbg.contains(&rival_dbg),
                            "seat {s} observation leaked rival seat {r}'s mandate"
                        );
                    }
                }
            }
        }

        let mut terminal = must_ok(Terminal::new(TestBackend::new(80, 10)), "TestBackend");
        must_ok(
            terminal.draw(|frame| {
                must_ok(
                    session.render_seat_frame(frame, Seat(0), false),
                    "render seat 0",
                );
            }),
            "draw seat 0",
        );
        let text_std = screen_text(&terminal);
        assert!(text_std.contains("Table #81001 | Turn 1"));
        assert!(text_std.contains("Seat 0 View"));

        let receipt = must_ok(
            session.commit_reveal_and_sweep(&BTreeMap::new()),
            "commit reveal sweep",
        );
        assert_eq!(receipt.turn, 1);
        assert_eq!(session.turn(), 2);
        assert_eq!(session.public_receipts().len(), 1);

        let dir = std::env::temp_dir().join(format!("ml_tui_host_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let save_path = dir.join("tui-session.save.json");
        let seal_path = dir.join("tui-session.seal");
        must_ok(
            session.write_save_and_seal(&save_path, &seal_path),
            "write save and seal",
        );
        let reloaded = must_ok(
            TuiHostSession::read_save_and_seal(&save_path, &seal_path, None),
            "reload",
        );
        assert_eq!(reloaded.turn(), 2);
        assert_eq!(reloaded.public_receipts().len(), 1);
        must_ok(TuiHostSession::wipe_seal_file(&seal_path), "wipe seal");
        assert!(!seal_path.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn host_session_from_ring_table_renders_commodity_names_and_plain_codes() {
        let ring = dev_ring_table(1);
        let session = must_ok(
            TuiHostSession::from_ring_table(&ring, TableId(81_002), &[0], 11),
            "ring table session",
        );
        let mut terminal = must_ok(Terminal::new(TestBackend::new(84, 10)), "TestBackend");
        must_ok(
            terminal.draw(|frame| {
                must_ok(
                    session.render_seat_frame(frame, Seat(0), false),
                    "render in-world names",
                );
            }),
            "draw in-world names",
        );
        let in_world = screen_text(&terminal);
        assert!(in_world.contains("Clearinghouse Marks [CRED]"));
        assert!(in_world.contains('['));

        must_ok(
            terminal.draw(|frame| {
                must_ok(
                    session.render_seat_frame(frame, Seat(0), true),
                    "render plain codes",
                );
            }),
            "draw plain codes",
        );
        let plain = screen_text(&terminal);
        assert!(plain.contains("CRED:"));
        assert!(!plain.contains("Clearinghouse Marks [CRED]"));
    }
}
