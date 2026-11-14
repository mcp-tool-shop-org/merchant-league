"""Look up every hash from a probe on the Testnet, independently of the Rust client.

Usage: python -I -X utf8 hash_check.py <01_probe.out> <02_hash_check.json>

For each step with a hash, it records whether the server knows the
transaction, whether it is in a validated ledger, its result and its ledger
index. It then records whether that matches what the probe printed: a
validated step must have the same result and ledger index, and a step that
never validated (a tem result) must not be in any validated ledger.
"""

import json
import sys
import urllib.request

RPC = "https://s.altnet.rippletest.net:51234"


def call(method, params):
    body = json.dumps({"method": method, "params": [params]}).encode()
    req = urllib.request.Request(RPC, body, {"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=30) as r:
        return json.load(r)["result"]


info = call("server_info", {})
assert info["info"]["network_id"] == 1, info["info"].get("network_id")

steps = [json.loads(line) for line in open(sys.argv[1], encoding="utf-8")]
checks = []
for s in steps:
    if "hash" not in s:
        continue
    tx = call("tx", {"transaction": s["hash"], "api_version": 2})
    found = "error" not in tx
    row = {
        "step": s["step"],
        "hash": s["hash"],
        "found": found,
        "validated": tx.get("validated") if found else None,
        "result": tx.get("meta", {}).get("TransactionResult") if found else tx.get("error"),
        "ledger_index": tx.get("ledger_index") if found else None,
    }
    row["matches_proof"] = (
        row["result"] == s["result"] and row["ledger_index"] == s["ledger_index"]
        if s["validated"]
        else not (found and tx.get("validated"))
    )
    if found:
        row["type"] = tx.get("tx_json", tx).get("TransactionType")
    checks.append(row)

json.dump(
    {"server": {"network_id": 1, "build": info["info"]["build_version"]}, "checks": checks},
    open(sys.argv[2], "w", encoding="utf-8"),
    indent=2,
)
bad = [c["step"] for c in checks if not c["matches_proof"]]
print(f"{len(checks)} hashes checked; mismatches: {bad}")
