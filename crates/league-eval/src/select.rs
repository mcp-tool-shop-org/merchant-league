//! Deterministic stratified state selection (`league-eval select`, `D5`, `Item 14`).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::driver::write_json_pretty_atomic;
use crate::error::EvalError;
use crate::records::EvalManifest;

/// Stratum bucket (`early`, `mid`, `late`) corresponding to thirds of a game's played turns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnStratum {
    /// First third of played turns (`0..n/3`).
    Early,
    /// Middle third of played turns.
    Mid,
    /// Final third of played turns.
    Late,
}

/// One frozen `(game, day, seat)` target emitted by `league-eval select` (`Item 14`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectedPreActState {
    /// Game directory containing `checkpoints/day-DD-pre-act.{save.json,seal}`.
    pub game_dir: PathBuf,
    /// Source `game_id` (`R5`) recorded in the game's `manifest.json`.
    pub game_id: String,
    /// Selected 1-based turn index (`day`).
    pub day: u8,
    /// Stratum bucket (`early`, `mid`, `late`).
    pub stratum: TurnStratum,
    /// Zero-based focal seat index (`0..4`).
    pub seat_index: u8,
    /// Focal seat guild name.
    pub seat_guild: String,
    /// Prefix `<game_dir>/checkpoints/day-DD-pre-act` read directly by `branch`.
    pub checkpoint_prefix: PathBuf,
}

/// Complete deterministic selection manifest written by `league-eval select --out <list.json>`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectionList {
    /// Format version (`1`).
    pub version: u32,
    /// Selection RNG seed supplied via `--selection-seed`.
    pub selection_seed: u64,
    /// Total states requested per game (`--per-game`, default `6`).
    pub per_game: usize,
    /// Early / mid / late counts (`--strata`, default `[2, 2, 2]`).
    pub strata: [usize; 3],
    /// Ordered selected states (`games.len() * per_game`).
    pub states: Vec<SelectedPreActState>,
}

/// Arguments for [`run_select`].
#[derive(Debug, Clone)]
pub struct SelectOptions {
    /// Input game directories (`--games <dir>...`).
    pub game_dirs: Vec<PathBuf>,
    /// States to pick per game (`--per-game`, default `6`).
    pub per_game: usize,
    /// Early/mid/late counts (`--strata`, default `[2, 2, 2]`).
    pub strata: [usize; 3],
    /// Deterministic seed (`--selection-seed`).
    pub selection_seed: u64,
    /// Output JSON path (`--out`).
    pub out_path: PathBuf,
}

/// Deterministically select stratified `(game, day, seat)` pre-act states across `opts.game_dirs`.
pub fn run_select(opts: &SelectOptions) -> Result<SelectionList, EvalError> {
    if opts.game_dirs.is_empty() {
        return Err(EvalError::Usage(
            "select requires at least one `--games <dir>` argument".to_owned(),
        ));
    }
    let strata_sum = opts.strata[0] + opts.strata[1] + opts.strata[2];
    if strata_sum != opts.per_game || opts.per_game == 0 {
        return Err(EvalError::InvalidStrata(format!(
            "strata {:?} sums to {strata_sum}, expected `--per-game {}` (> 0)",
            opts.strata, opts.per_game
        )));
    }

    let mut states = Vec::with_capacity(opts.game_dirs.len() * opts.per_game);
    for (game_ordinal, game_dir) in opts.game_dirs.iter().enumerate() {
        let manifest_path = game_dir.join("manifest.json");
        let raw = fs::read_to_string(&manifest_path).map_err(|e| {
            EvalError::CheckpointNotFound(format!(
                "missing game manifest {}: {e}",
                manifest_path.display()
            ))
        })?;
        let manifest: EvalManifest = serde_json::from_str(&raw).map_err(|e| {
            EvalError::Io(format!(
                "invalid manifest JSON in {}: {e}",
                manifest_path.display()
            ))
        })?;
        if manifest.seats.len() != 4 {
            return Err(EvalError::InvalidSeatCount {
                actual: manifest.seats.len(),
            });
        }

        let ckpts_dir = game_dir.join("checkpoints");
        let played_days = discover_playable_days(&ckpts_dir)?;
        if played_days.len() < opts.per_game {
            return Err(EvalError::InsufficientTurnsForStrata(format!(
                "game {} has {} pre-act+recorded checkpoints, fewer than `--per-game {}`",
                game_dir.display(),
                played_days.len(),
                opts.per_game
            )));
        }

        let [early_days, mid_days, late_days] = partition_thirds(&played_days);
        let buckets = [
            (TurnStratum::Early, early_days, opts.strata[0]),
            (TurnStratum::Mid, mid_days, opts.strata[1]),
            (TurnStratum::Late, late_days, opts.strata[2]),
        ];

        for (stratum_idx, (stratum, pool, need)) in buckets.into_iter().enumerate() {
            if pool.len() < need {
                return Err(EvalError::InsufficientTurnsForStrata(format!(
                    "game {} stratum {:?} has {} turn(s), fewer than requested {need}",
                    game_dir.display(),
                    stratum,
                    pool.len()
                )));
            }
            let picked_indices = deterministic_pick_distinct(
                opts.selection_seed,
                game_ordinal,
                stratum_idx,
                pool.len(),
                need,
            );
            let mut picked_days: Vec<u8> = picked_indices.into_iter().map(|i| pool[i]).collect();
            picked_days.sort_unstable();
            for (pick_rank, day) in picked_days.into_iter().enumerate() {
                let seat_index = deterministic_pick_seat(
                    opts.selection_seed,
                    game_ordinal,
                    stratum_idx,
                    pick_rank,
                    day,
                );
                let seat = &manifest.seats[usize::from(seat_index)];
                let checkpoint_prefix = ckpts_dir.join(format!("day-{day:02}-pre-act"));
                states.push(SelectedPreActState {
                    game_dir: game_dir.clone(),
                    game_id: manifest.game_id.clone(),
                    day,
                    stratum,
                    seat_index,
                    seat_guild: seat.guild.clone(),
                    checkpoint_prefix,
                });
            }
        }
    }

    let list = SelectionList {
        version: 1,
        selection_seed: opts.selection_seed,
        per_game: opts.per_game,
        strata: opts.strata,
        states,
    };
    write_json_pretty_atomic(&opts.out_path, &list)?;
    Ok(list)
}

fn discover_playable_days(checkpoints_dir: &Path) -> Result<Vec<u8>, EvalError> {
    let entries = fs::read_dir(checkpoints_dir).map_err(|e| {
        EvalError::CheckpointNotFound(format!(
            "cannot read checkpoints dir {}: {e}",
            checkpoints_dir.display()
        ))
    })?;
    let mut flags: BTreeMap<u8, (bool, bool, bool)> = BTreeMap::new();
    for entry in entries {
        let entry = entry.map_err(|e| EvalError::Io(e.to_string()))?;
        let name = entry.file_name();
        let Some(name_str) = name.to_str() else {
            continue;
        };
        if let Some(rest) = name_str.strip_prefix("day-")
            && rest.len() >= 2
            && let Ok(day) = rest[..2].parse::<u8>()
        {
            let slot = flags.entry(day).or_insert((false, false, false));
            match &rest[2..] {
                "-pre-act.save.json" => slot.0 = true,
                "-pre-act.seal" => slot.1 = true,
                "-act-recorded.seal" => slot.2 = true,
                _ => {}
            }
        }
    }
    let days: Vec<u8> = flags
        .into_iter()
        .filter_map(|(day, (a, b, c))| (a && b && c).then_some(day))
        .collect();
    Ok(days)
}

/// Partition sorted `days` cleanly into `[early, mid, late]` thirds preserving chronological order.
#[must_use]
pub fn partition_thirds(days: &[u8]) -> [Vec<u8>; 3] {
    let n = days.len();
    let base = n / 3;
    let rem = n % 3;
    let c0 = base + usize::from(rem > 0);
    let c1 = base + usize::from(rem > 1);
    let early = days[..c0].to_vec();
    let mid = days[c0..c0 + c1].to_vec();
    let late = days[c0 + c1..].to_vec();
    [early, mid, late]
}

fn hash_u64(seed: u64, domain: &[u8], parts: &[u64]) -> u64 {
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update(seed.to_le_bytes());
    for &p in parts {
        hasher.update(p.to_le_bytes());
    }
    let digest: [u8; 32] = hasher.finalize().into();
    let mut arr = [0_u8; 8];
    arr.copy_from_slice(&digest[..8]);
    u64::from_le_bytes(arr)
}

fn deterministic_pick_distinct(
    seed: u64,
    game_ordinal: usize,
    stratum_idx: usize,
    pool_len: usize,
    need: usize,
) -> Vec<usize> {
    let mut indices: Vec<usize> = (0..pool_len).collect();
    for step in 0..need {
        let remaining = pool_len - step;
        let roll = hash_u64(
            seed,
            b"league-eval/select-turn/v1",
            &[game_ordinal as u64, stratum_idx as u64, step as u64],
        );
        let chosen_offset = (roll as usize) % remaining;
        indices.swap(step, step + chosen_offset);
    }
    indices.truncate(need);
    indices
}

fn deterministic_pick_seat(
    seed: u64,
    game_ordinal: usize,
    stratum_idx: usize,
    pick_rank: usize,
    day: u8,
) -> u8 {
    let roll = hash_u64(
        seed,
        b"league-eval/select-seat/v1",
        &[
            game_ordinal as u64,
            stratum_idx as u64,
            pick_rank as u64,
            u64::from(day),
        ],
    );
    (roll % 4) as u8
}
