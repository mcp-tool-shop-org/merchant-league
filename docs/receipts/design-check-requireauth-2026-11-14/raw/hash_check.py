"""Look up every hash from the proof on the Testnet, independently of the Rust client.

Usage: python -I -X utf8 hash_check.py <01_proof.out> <02_hash_check.json> <vectors-out.json>

Writes a per-step check (validated, result matches, ledger index matches) and
a vector file of the validated TokenEscrow and DomainID transactions
(tx_json as the server returns it, plus tx_blob from a binary lookup).
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

steps = [json.loads(l) for l in open(sys.argv[1], encoding="utf-8")]
checks = []
vectors = []
KEEP = {
    "token_escrow_without_locking_flag",
    "token_escrow_create",
    "token_escrow_finish_by_trader",
    "issuer_domain_set",
    "merchant_domain_offer_sell_1",
    "outsider_domain_offer_refused",
    "trader_domain_offer_buy_1",
    "merchant_domain_payment_to_trader",
    "issuer_allow_trustline_locking",
}
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
    if s["step"] in KEEP and found and tx.get("validated"):
        binary = call("tx", {"transaction": s["hash"], "binary": True, "api_version": 2})
        tx_json = tx.get("tx_json", {})
        vectors.append(
            {
                "step": s["step"],
                "hash": s["hash"],
                "ledger_index": tx.get("ledger_index"),
                "validated": True,
                "result": tx["meta"]["TransactionResult"],
                "tx_json": tx_json,
                "tx_blob": binary.get("tx_blob"),
            }
        )

json.dump(
    {"server": {"network_id": 1, "build": info["info"]["build_version"]}, "checks": checks},
    open(sys.argv[2], "w", encoding="utf-8"),
    indent=2,
)
json.dump(
    {
        "source": "Validated on the XRPL Testnet on 2026-11-14 by the league's M2c proof "
        "(docs/receipts/m2c-patched-xrpl-rust-proof-2026-11-14); tx_json and tx_blob as the server returned them.",
        "network_id": 1,
        "transactions": vectors,
    },
    open(sys.argv[3], "w", encoding="utf-8"),
    indent=2,
)
bad = [c["step"] for c in checks if not c["matches_proof"]]
print(f"{len(checks)} hashes checked; {len(vectors)} vectors; mismatches: {bad}")
