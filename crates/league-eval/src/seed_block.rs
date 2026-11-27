//! Deterministic 4-seat Latin-square seed-block manifest generator (`league-eval seed-block`).
//!
//! Generates seat-rotated duplicate match specifications (`4` rotations per layout seed so every
//! entrant plays every table seat `0..4` cleanly without bias), keeping `D5`'s single-state
//! `branch` evaluation separate from cross-seat full-season blocks.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use league_core::{HARD_TURN_CAP, PolicyChaChaRng};
use league_relay::{ApiKey, SeatBackend, SeatFile, sha256_hex};
use league_scorer::to_sorted_pretty_json;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::driver::{validate_eval_seats_for_communication, write_text_atomic};
use crate::error::EvalError;
use crate::mechanisms::parse_json_prose_safe;
use crate::records::{CommunicationMode, EvalArm};

/// Schema version for [`SeedBlockManifest`].
pub const SEED_BLOCK_SCHEMA_VERSION: u32 = 1;

/// Default deterministic RNG seed for Latin-square seat permutations.
pub const DEFAULT_SEED_BLOCK_RNG_SEED: u64 = 20_261_127;

/// Domain separation tag for per-seed 4×4 Latin square generation.
pub const SEED_BLOCK_RNG_DOMAIN: &[u8] = b"league-eval/seed-block-latin-square/v1";

/// Arguments for [`run_seed_block`].
#[derive(Debug, Clone)]
pub struct SeedBlockOptions {
    /// Table layout seeds to expand into 4-seat rotational blocks (`--seeds <u64>...`).
    pub seeds: Vec<u64>,
    /// Path to 4-seat `seats.json` defining the 4 entrants (`--seats <file>`).
    pub seats_path: PathBuf,
    /// Configured Evaluation arm recorded in the manifest (`--arm <off|control|on>`).
    pub arm: EvalArm,
    /// Communication condition (`--communication <talk|silent>`).
    pub communication: CommunicationMode,
    /// Turn cap recorded per block (`--turns <1..=32>`, default `HARD_TURN_CAP`).
    pub turns: u8,
    /// Starting table identifier allocated sequentially across blocks (`--table-id-base <u64>`).
    pub table_id_base: u64,
    /// Deterministic RNG seed governing per-seed Latin squares (`--block-seed <u64>`).
    pub block_seed: u64,
    /// Output directory where `seed_block_manifest.json` and `seeds/...` are written (`--out <dir>`).
    pub out_dir: PathBuf,
    /// Optional VM hosts forwarded to `SeatFile::load_with_allowed_hosts`.
    pub allowed_vm_hosts: Vec<String>,
}

/// Summary of one configured entrant in the base `--seats` file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeedBlockEntrant {
    /// Zero-based index of this entrant in the base `--seats` file (`0..4`).
    pub entrant_index: u8,
    /// Original guild label from the base `--seats` file.
    pub label_guild: String,
    /// Backend label (`"file"` or `"local"`).
    pub backend: String,
    /// Configured model or scripted baseline identifier.
    pub model: String,
}

/// One seat-rotated match specification inside a seed block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeedBlockGameSpec {
    /// Zero-based rotation index (`0..4`).
    pub rotation_index: u8,
    /// Relative game directory suggested for `league-eval play` (`seeds/seed-<n>/rot-<r>`).
    pub game_dir: String,
    /// Relative path to the rotated `rot-<r>.seats.json` file under `--out`.
    pub seats_file: String,
    /// Lowercase hex SHA-256 of the generated `rot-<r>.seats.json` file bytes.
    pub seats_sha256: String,
    /// Entrant indices (`0..4`) placed at table seats `[0, 1, 2, 3]` in this rotation.
    pub seat_entrants: [u8; 4],
    /// Base entrant label guilds assigned to seats `[0, 1, 2, 3]` in this rotation.
    pub seat_entrant_guilds: [String; 4],
}

/// One seed's 4×4 rotational duplicate block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeedBlockEntry {
    /// Table layout seed.
    pub seed: u64,
    /// Assigned table identifier for this seed block.
    pub table_id: u64,
    /// Relative Turn-0 duplicate checkpoint directory (`seeds/seed-<n>/turn0`).
    pub turn0_dir: String,
    /// Validated 4×4 Latin square (`latin_square[rotation][table_seat] = entrant_index`).
    pub latin_square: [[u8; 4]; 4],
    /// Four rotational games belonging to this seed block.
    pub games: Vec<SeedBlockGameSpec>,
}

/// Top-level manifest written to `<out>/seed_block_manifest.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeedBlockManifest {
    /// Schema version (`1`).
    pub schema_version: u32,
    /// RNG seed used to derive each seed's Latin square.
    pub block_seed: u64,
    /// Communication condition (`talk` omitted on JSON; `silent` serialized).
    #[serde(default, skip_serializing_if = "CommunicationMode::is_talk")]
    pub communication: CommunicationMode,
    /// Recorded Evaluation arm (`off`, `control`, `on`).
    pub arm: EvalArm,
    /// Turn cap recorded per game (`1..=32`).
    pub turns: u8,
    /// Lowercase hex SHA-256 of the source `--seats` file.
    pub source_seats_sha256: String,
    /// Canonical table guild names `[seat_0, seat_1, seat_2, seat_3]` preserved across rotations.
    pub table_guilds: [String; 4],
    /// Ordered list of the 4 entrants from `--seats`.
    pub entrants: Vec<SeedBlockEntrant>,
    /// Generated per-seed rotational duplicate blocks.
    pub blocks: Vec<SeedBlockEntry>,
}

fn shuffle_4(rng: &mut PolicyChaChaRng) -> [u8; 4] {
    let mut arr = [0_u8, 1, 2, 3];
    for i in (1..4).rev() {
        let j = rng.gen_index(i + 1);
        arr.swap(i, j);
    }
    arr
}

/// Deterministically construct a 4×4 Latin square keyed by `(block_seed, layout_seed)`.
#[must_use]
pub fn build_seed_latin_square(block_seed: u64, layout_seed: u64) -> [[u8; 4]; 4] {
    let mut hasher = Sha256::new();
    hasher.update(SEED_BLOCK_RNG_DOMAIN);
    hasher.update(block_seed.to_le_bytes());
    hasher.update(layout_seed.to_le_bytes());
    let digest: [u8; 32] = hasher.finalize().into();
    let mut rng = PolicyChaChaRng::from_seed_bytes(digest);

    let entrant_perm = shuffle_4(&mut rng);
    let row_shifts = shuffle_4(&mut rng);
    let mut square = [[0_u8; 4]; 4];
    for r in 0..4 {
        let shift = usize::from(row_shifts[r]);
        for c in 0..4 {
            square[r][c] = entrant_perm[(c + shift) % 4];
        }
    }
    square
}

/// Returns `true` iff `square` is a valid 4×4 Latin square over `{0, 1, 2, 3}`.
#[must_use]
pub fn verify_latin_square_4x4(square: &[[u8; 4]; 4]) -> bool {
    for row in square {
        let mut seen = [false; 4];
        for &val in row {
            let idx = usize::from(val);
            if idx >= 4 || seen[idx] {
                return false;
            }
            seen[idx] = true;
        }
    }
    for col in 0..4 {
        let mut seen = [false; 4];
        for row in square {
            let idx = usize::from(row[col]);
            if idx >= 4 || seen[idx] {
                return false;
            }
            seen[idx] = true;
        }
    }
    true
}

type RawSeatsJson = (Option<Map<String, Value>>, Vec<Value>);

fn parse_raw_seats_array(raw_json: &str, path: &Path) -> Result<RawSeatsJson, EvalError> {
    let val: Value = parse_json_prose_safe(raw_json, path)?;
    match val {
        Value::Array(items) => Ok((None, items)),
        Value::Object(mut obj) => {
            let seats_val = obj.remove("seats").ok_or_else(|| {
                EvalError::SchemaValidation(format!(
                    "seats file {} missing `seats` array",
                    path.display()
                ))
            })?;
            let Value::Array(items) = seats_val else {
                return Err(EvalError::SchemaValidation(format!(
                    "seats file {} `seats` is not an array",
                    path.display()
                )));
            };
            Ok((Some(obj), items))
        }
        _ => Err(EvalError::SchemaValidation(format!(
            "seats file {} must be a JSON array or object",
            path.display()
        ))),
    }
}

/// Generate a deterministic 4-seat Latin-square seed-block manifest and rotated `*.seats.json` files.
pub fn run_seed_block(
    opts: &SeedBlockOptions,
    redaction_key: Option<&ApiKey>,
) -> Result<SeedBlockManifest, EvalError> {
    if opts.seeds.is_empty() {
        return Err(EvalError::Usage(
            "`seed-block` requires at least one seed via `--seeds`".to_owned(),
        ));
    }
    if opts.turns == 0 || opts.turns > HARD_TURN_CAP {
        return Err(EvalError::Usage(format!(
            "`--turns` must be in 1..={HARD_TURN_CAP}, got {}",
            opts.turns
        )));
    }
    let mut unique_seeds = BTreeSet::new();
    for &seed in &opts.seeds {
        if !unique_seeds.insert(seed) {
            return Err(EvalError::Usage(format!(
                "duplicate layout seed {seed} in `--seeds`"
            )));
        }
    }
    if opts
        .table_id_base
        .checked_add(u64::try_from(opts.seeds.len()).unwrap_or(u64::MAX))
        .is_none()
    {
        return Err(EvalError::Usage(
            "`--table-id-base` + seed count overflows u64".to_owned(),
        ));
    }

    let manifest_path = opts.out_dir.join("seed_block_manifest.json");
    if manifest_path.exists() {
        return Err(EvalError::Usage(format!(
            "refusing to overwrite existing seed-block manifest {}",
            manifest_path.display()
        )));
    }

    // Validate with league-relay's secret-aware loader BEFORE inspecting JSON structure.
    let seats_file =
        SeatFile::load_with_allowed_hosts(&opts.seats_path, redaction_key, &opts.allowed_vm_hosts)?;
    validate_eval_seats_for_communication(&seats_file, opts.communication)?;

    let raw_seats = fs::read_to_string(&opts.seats_path)
        .map_err(|e| EvalError::Io(format!("read {}: {e}", opts.seats_path.display())))?;
    let source_seats_sha256 = sha256_hex(raw_seats.as_bytes());
    let (wrapper_meta, raw_seat_items) = parse_raw_seats_array(&raw_seats, &opts.seats_path)?;
    if raw_seat_items.len() != 4 {
        return Err(EvalError::InvalidSeatCount {
            actual: raw_seat_items.len(),
        });
    }

    let table_guilds = [
        seats_file.seats[0].guild.clone(),
        seats_file.seats[1].guild.clone(),
        seats_file.seats[2].guild.clone(),
        seats_file.seats[3].guild.clone(),
    ];
    let entrants: Vec<SeedBlockEntrant> = seats_file
        .seats
        .iter()
        .enumerate()
        .map(|(idx, s)| {
            let (backend, model) = match &s.backend {
                SeatBackend::File { model } => (
                    "file".to_owned(),
                    model.clone().unwrap_or_else(|| "ClubBot".to_owned()),
                ),
                SeatBackend::Local(local) => ("local".to_owned(), local.model.clone()),
                SeatBackend::Openrouter(or_seat) => {
                    ("openrouter".to_owned(), or_seat.model.clone())
                }
            };
            SeedBlockEntrant {
                entrant_index: idx as u8,
                label_guild: s.guild.clone(),
                backend,
                model,
            }
        })
        .collect();

    // Pre-build and validate every rotated seat file in memory before touching disk.
    let mut pending_writes: Vec<(PathBuf, String)> = Vec::new();
    let mut blocks = Vec::with_capacity(opts.seeds.len());
    for (seed_ord, &seed) in opts.seeds.iter().enumerate() {
        let table_id = opts.table_id_base + (seed_ord as u64);
        let square = build_seed_latin_square(opts.block_seed, seed);
        debug_assert!(verify_latin_square_4x4(&square));

        let mut games = Vec::with_capacity(4);
        for (rot_idx, row) in square.iter().enumerate() {
            let mut rotated_items = Vec::with_capacity(4);
            for (table_seat_idx, &entrant_idx) in row.iter().enumerate() {
                let mut item_obj = match raw_seat_items[usize::from(entrant_idx)].as_object() {
                    Some(obj) => obj.clone(),
                    None => {
                        return Err(EvalError::SchemaValidation(
                            "seat entry is not a JSON object".to_owned(),
                        ));
                    }
                };
                item_obj.insert(
                    "guild".to_owned(),
                    Value::String(table_guilds[table_seat_idx].clone()),
                );
                rotated_items.push(Value::Object(item_obj));
            }
            let rotated_val = match &wrapper_meta {
                Some(meta) => {
                    let mut root = meta.clone();
                    root.insert("seats".to_owned(), Value::Array(rotated_items));
                    Value::Object(root)
                }
                None => Value::Array(rotated_items),
            };
            let rotated_json = format!("{}\n", to_sorted_pretty_json(&rotated_val)?);
            let check_loaded = SeatFile::from_json_str_with_allowed_hosts(
                &rotated_json,
                redaction_key,
                &opts.allowed_vm_hosts,
            )?;
            validate_eval_seats_for_communication(&check_loaded, opts.communication)?;

            let seats_sha256 = sha256_hex(rotated_json.as_bytes());
            let rel_seats = format!("seeds/seed-{seed}/rot-{rot_idx}.seats.json");
            let rel_game_dir = format!("seeds/seed-{seed}/rot-{rot_idx}");
            pending_writes.push((opts.out_dir.join(&rel_seats), rotated_json));

            games.push(SeedBlockGameSpec {
                rotation_index: rot_idx as u8,
                game_dir: rel_game_dir,
                seats_file: rel_seats,
                seats_sha256,
                seat_entrants: *row,
                seat_entrant_guilds: [
                    entrants[usize::from(row[0])].label_guild.clone(),
                    entrants[usize::from(row[1])].label_guild.clone(),
                    entrants[usize::from(row[2])].label_guild.clone(),
                    entrants[usize::from(row[3])].label_guild.clone(),
                ],
            });
        }

        blocks.push(SeedBlockEntry {
            seed,
            table_id,
            turn0_dir: format!("seeds/seed-{seed}/turn0"),
            latin_square: square,
            games,
        });
    }

    let manifest = SeedBlockManifest {
        schema_version: SEED_BLOCK_SCHEMA_VERSION,
        block_seed: opts.block_seed,
        communication: opts.communication,
        arm: opts.arm,
        turns: opts.turns,
        source_seats_sha256,
        table_guilds,
        entrants,
        blocks,
    };
    let manifest_json = format!("{}\n", to_sorted_pretty_json(&manifest)?);

    for (path, content) in pending_writes {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| {
                EvalError::Io(format!("create seed-block dir {}: {e}", parent.display()))
            })?;
        }
        write_text_atomic(&path, &content)?;
    }
    write_text_atomic(&manifest_path, &manifest_json)?;
    Ok(manifest)
}
