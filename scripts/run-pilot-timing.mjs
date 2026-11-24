#!/usr/bin/env node
/**
 * run-pilot-timing.mjs — multi-process release timing runner for `league-scorer`.
 *
 * Builds the `timing_bench` release test binary once, generates a shared checkpoint
 * fixture, executes N independent worker processes sequentially (measuring cold first
 * rollout vs warm steady state, 5-stage breakdowns, per-decision-point and per-rollout
 * totals, and 1-vs-N scoped-thread scaling), writes one JSON line per process to
 * `<out>.jsonl`, and emits a multi-process median + spread summary to `<out>.summary.json`.
 *
 * Usage:
 *   node scripts/run-pilot-timing.mjs [--processes 5] [--smoke] [--out <prefix>] [--jobs 2] [--timeout-secs 600]
 */
import { spawnSync } from 'node:child_process';
import {
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const SCRIPT_DIR = path.dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = path.resolve(SCRIPT_DIR, '..');

const PROVIDER_KEY_VARS = [
  'OPENROUTER_API_KEY',
  'OPENAI_API_KEY',
  'ANTHROPIC_API_KEY',
  'GEMINI_API_KEY',
];

function cleanEnv(extra = {}) {
  const env = { ...process.env, ...extra };
  for (const key of PROVIDER_KEY_VARS) {
    delete env[key];
  }
  return env;
}

function parsePositiveInt(raw, flag) {
  if (!/^[1-9]\d*$/.test(String(raw ?? ''))) {
    throw new Error(`${flag} must be a positive integer, got ${JSON.stringify(raw)}`);
  }
  return Number(raw);
}

function printHelp() {
  console.log(
    [
      'Usage: node scripts/run-pilot-timing.mjs [options]',
      '',
      'Options:',
      '  --processes <N>      Number of independent worker processes (default: 5)',
      '  --smoke              Smallest checkpoint/candidate set (fast smoke run)',
      '  --out <prefix>       Output path prefix for <prefix>.jsonl and <prefix>.summary.json',
      '  --jobs <J>           Cargo build parallelism passed via --jobs (default: 2)',
      '  --timeout-secs <S>   Per-command timeout in seconds (default: 600)',
      '  -h, --help           Show this help message',
    ].join('\n')
  );
}

function parseArgs(argv) {
  const opts = {
    processes: 5,
    smoke: false,
    outPrefix: path.join(os.tmpdir(), `ml-pilot-timing-${process.pid}`),
    jobs: 2,
    timeoutSecs: 600,
    help: false,
  };

  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === '--help' || arg === '-h') {
      opts.help = true;
    } else if (arg === '--smoke') {
      opts.smoke = true;
    } else if (arg === '--processes') {
      i += 1;
      opts.processes = parsePositiveInt(argv[i], '--processes');
    } else if (arg === '--out') {
      i += 1;
      if (!argv[i]) {
        throw new Error('--out requires a non-empty path prefix');
      }
      opts.outPrefix = path.resolve(argv[i]);
    } else if (arg === '--jobs') {
      i += 1;
      opts.jobs = parsePositiveInt(argv[i], '--jobs');
    } else if (arg === '--timeout-secs') {
      i += 1;
      opts.timeoutSecs = parsePositiveInt(argv[i], '--timeout-secs');
    } else {
      throw new Error(`Unknown option: ${arg}`);
    }
  }
  return opts;
}

function runChecked(cmd, args, extraEnv, timeoutSecs, label) {
  const res = spawnSync(cmd, args, {
    cwd: REPO_ROOT,
    env: cleanEnv(extraEnv),
    stdio: ['ignore', 'pipe', 'pipe'],
    encoding: 'utf8',
    timeout: timeoutSecs * 1000,
    maxBuffer: 32 * 1024 * 1024,
  });
  if (res.error) {
    throw new Error(`${label} failed to run: ${res.error.message}`);
  }
  if (res.status !== 0) {
    const stderrTail = (res.stderr || '').trim().split('\n').slice(-20).join('\n');
    const stdoutTail = (res.stdout || '').trim().split('\n').slice(-20).join('\n');
    throw new Error(
      `${label} exited with code ${res.status}\nstdout:\n${stdoutTail}\nstderr:\n${stderrTail}`
    );
  }
  return res;
}

function buildTimingBenchBinary(jobs, timeoutSecs) {
  const cargoCmd = process.platform === 'win32' ? 'cargo.exe' : 'cargo';
  const res = runChecked(
    cargoCmd,
    [
      'test',
      '--release',
      '--locked',
      '--jobs',
      String(jobs),
      '-p',
      'league-scorer',
      '--test',
      'timing_bench',
      '--no-run',
      '--message-format=json',
    ],
    {},
    timeoutSecs,
    'cargo build timing_bench (--release)'
  );

  let executable = null;
  for (const line of (res.stdout || '').split('\n')) {
    const trimmed = line.trim();
    if (!trimmed.startsWith('{')) {
      continue;
    }
    let msg;
    try {
      msg = JSON.parse(trimmed);
    } catch {
      continue;
    }
    if (
      msg.reason === 'compiler-artifact' &&
      msg.target?.name === 'timing_bench' &&
      msg.profile?.test === true &&
      typeof msg.executable === 'string' &&
      msg.executable.length > 0
    ) {
      executable = msg.executable;
    }
  }
  if (!executable) {
    throw new Error('Could not locate compiled timing_bench test executable from cargo JSON output');
  }
  return executable;
}

function main() {
  const opts = parseArgs(process.argv.slice(2));
  if (opts.help) {
    printHelp();
    return;
  }

  const outDir = path.dirname(opts.outPrefix);
  mkdirSync(outDir, { recursive: true });
  const jsonlPath = `${opts.outPrefix}.jsonl`;
  const summaryPath = `${opts.outPrefix}.summary.json`;

  const benchExe = buildTimingBenchBinary(opts.jobs, opts.timeoutSecs);
  const fixtureDir = mkdtempSync(path.join(os.tmpdir(), 'ml_pilot_timing_fixture_'));
  const benchArgs = [
    '--exact',
    'timing_benchmark_early_mid_late_rollouts',
    '--ignored',
    '--nocapture',
  ];

  try {
    const smokeEnv = opts.smoke ? { LEAGUE_TIMING_SMOKE: '1' } : {};

    runChecked(
      benchExe,
      benchArgs,
      {
        ...smokeEnv,
        LEAGUE_TIMING_MODE: 'setup',
        LEAGUE_TIMING_FIXTURE_DIR: fixtureDir,
      },
      opts.timeoutSecs,
      'timing_bench setup'
    );

    const lines = [];
    for (let procIdx = 0; procIdx < opts.processes; procIdx += 1) {
      const workerOut = path.join(fixtureDir, `worker_${procIdx}.json`);
      runChecked(
        benchExe,
        benchArgs,
        {
          ...smokeEnv,
          LEAGUE_TIMING_MODE: 'worker',
          LEAGUE_TIMING_FIXTURE_DIR: fixtureDir,
          LEAGUE_TIMING_PROCESS_INDEX: String(procIdx),
          LEAGUE_TIMING_WORKER_OUT: workerOut,
        },
        opts.timeoutSecs,
        `timing_bench worker ${procIdx}`
      );
      const rawLine = readFileSync(workerOut, 'utf8').trim();
      JSON.parse(rawLine);
      lines.push(rawLine);
    }

    writeFileSync(jsonlPath, `${lines.join('\n')}\n`, 'utf8');

    const aggRes = runChecked(
      benchExe,
      benchArgs,
      {
        ...smokeEnv,
        LEAGUE_TIMING_MODE: 'aggregate',
        LEAGUE_TIMING_JSONL_IN: jsonlPath,
        LEAGUE_TIMING_SUMMARY_OUT: summaryPath,
      },
      opts.timeoutSecs,
      'timing_bench aggregate'
    );

    if (aggRes.stdout && aggRes.stdout.trim()) {
      process.stdout.write(`${aggRes.stdout.trimEnd()}\n`);
    }
    if (aggRes.stderr && aggRes.stderr.trim()) {
      process.stdout.write(`${aggRes.stderr.trimEnd()}\n`);
    }
    process.stdout.write(`PILOT_TIMING_JSONL=${jsonlPath}\n`);
    process.stdout.write(`PILOT_TIMING_SUMMARY=${summaryPath}\n`);
  } finally {
    rmSync(fixtureDir, { recursive: true, force: true });
  }
}

try {
  main();
} catch (err) {
  console.error(err instanceof Error ? err.message : String(err));
  process.exit(1);
}
