#!/usr/bin/env python3
import argparse
import base64
import hashlib
import hmac
import json
import sys
import time
import urllib.error
import urllib.request


ZERO_HASH = "0x" + "00" * 32
EMPTY_REQUESTS_HASH = "0xe3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"


def rpc(url, method, params, token=None, timeout=30):
    headers = {"Content-Type": "application/json"}
    if token:
        headers["Authorization"] = f"Bearer {token}"
    data = json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).encode()
    request = urllib.request.Request(url, data=data, headers=headers)
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            payload = json.loads(response.read().decode())
    except urllib.error.HTTPError as error:
        body = error.read().decode(errors="replace")
        raise RuntimeError(f"{method} HTTP {error.code}: {body}") from error
    if "error" in payload:
        raise RuntimeError(f"{method} RPC error: {payload['error']}")
    return payload.get("result")


def rpc_batch(url, calls, token=None, timeout=60):
    if not calls:
        return []
    headers = {"Content-Type": "application/json"}
    if token:
        headers["Authorization"] = f"Bearer {token}"
    data = json.dumps(
        [
            {"jsonrpc": "2.0", "id": index, "method": method, "params": params}
            for index, (method, params) in enumerate(calls)
        ]
    ).encode()
    request = urllib.request.Request(url, data=data, headers=headers)
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            payload = json.loads(response.read().decode())
    except urllib.error.HTTPError as error:
        body = error.read().decode(errors="replace")
        raise RuntimeError(f"batch HTTP {error.code}: {body}") from error
    if not isinstance(payload, list):
        raise RuntimeError(f"batch returned non-list response: {payload}")
    by_id = {}
    for item in payload:
        if "error" in item:
            raise RuntimeError(f"batch item {item.get('id')} RPC error: {item['error']}")
        by_id[item["id"]] = item.get("result")
    return [by_id[index] for index in range(len(calls))]


def jwt_token(secret_path):
    secret_hex = open(secret_path, "r", encoding="utf-8").read().strip()
    if secret_hex.startswith("0x"):
        secret_hex = secret_hex[2:]
    secret = bytes.fromhex(secret_hex)

    def b64(data):
        return base64.urlsafe_b64encode(data).rstrip(b"=")

    header = b64(json.dumps({"alg": "HS256", "typ": "JWT"}, separators=(",", ":")).encode())
    payload = b64(json.dumps({"iat": int(time.time())}, separators=(",", ":")).encode())
    signature = b64(hmac.new(secret, header + b"." + payload, hashlib.sha256).digest())
    return (header + b"." + payload + b"." + signature).decode()


def hex_number(number):
    return hex(number)


def block_number(block):
    return int(block["number"], 16)


def collect_raw_transactions(block, raw_by_hash):
    raw_txs = []
    blob_hashes = []
    for tx in block["transactions"]:
        tx_hash = tx["hash"] if isinstance(tx, dict) else tx
        if isinstance(tx, dict):
            blob_hashes.extend(tx.get("blobVersionedHashes") or [])
        raw = raw_by_hash.get(tx_hash.lower())
        if not raw:
            raise RuntimeError(f"missing raw transaction {tx_hash}")
        raw_txs.append(raw)
    return raw_txs, blob_hashes


def execution_payload(block, raw_by_hash):
    requests_hash = block.get("requestsHash")
    if requests_hash and requests_hash.lower() != EMPTY_REQUESTS_HASH:
        raise RuntimeError(f"block {block['number']} has non-empty requestsHash {requests_hash}")

    raw_txs, blob_hashes = collect_raw_transactions(block, raw_by_hash)
    payload = {
        "parentHash": block["parentHash"],
        "feeRecipient": block["miner"],
        "stateRoot": block["stateRoot"],
        "receiptsRoot": block["receiptsRoot"],
        "logsBloom": block["logsBloom"],
        "prevRandao": block["mixHash"],
        "blockNumber": block["number"],
        "gasLimit": block["gasLimit"],
        "gasUsed": block["gasUsed"],
        "timestamp": block["timestamp"],
        "extraData": block["extraData"],
        "baseFeePerGas": block.get("baseFeePerGas", "0x0"),
        "blockHash": block["hash"],
        "transactions": raw_txs,
        "withdrawals": block.get("withdrawals") or [],
        "blobGasUsed": block.get("blobGasUsed", "0x0"),
        "excessBlobGas": block.get("excessBlobGas", "0x0"),
    }
    return payload, blob_hashes, block.get("parentBeaconBlockRoot") or ZERO_HASH


def forkchoice(args, block_hash, number):
    fcu = rpc(
        args.auth_rpc,
        "engine_forkchoiceUpdatedV3",
        [
            {
                "headBlockHash": block_hash,
                "safeBlockHash": ZERO_HASH,
                "finalizedBlockHash": ZERO_HASH,
            },
            None,
        ],
        token=jwt_token(args.jwt_secret),
        timeout=args.timeout,
    )
    payload_status = fcu.get("payloadStatus", {})
    if payload_status.get("status") != "VALID":
        raise RuntimeError(f"FCU block {number} returned {fcu}")

    local_after = rpc(args.local_rpc, "eth_getBlockByNumber", [hex_number(number), False])
    if not local_after or local_after.get("hash", "").lower() != block_hash.lower():
        raise RuntimeError(f"local block {number} did not become canonical after FCU")


def replay_block(args, public_block, raw_by_hash, do_fcu):
    number = block_number(public_block)

    if args.check_existing:
        local_block = rpc(args.local_rpc, "eth_getBlockByNumber", [hex_number(number), False])
        if local_block and local_block.get("hash", "").lower() == public_block["hash"].lower():
            return "skip", public_block["hash"]

    payload, blob_hashes, parent_beacon_root = execution_payload(public_block, raw_by_hash)
    if args.dry_run:
        return "dry-run", public_block["hash"]

    status = rpc(
        args.auth_rpc,
        "engine_newPayloadV4",
        [payload, blob_hashes, parent_beacon_root, []],
        token=jwt_token(args.jwt_secret),
        timeout=args.timeout,
    )
    if status.get("status") != "VALID":
        raise RuntimeError(f"newPayload block {number} returned {status}")

    if do_fcu:
        forkchoice(args, public_block["hash"], number)
        return "replayed", public_block["hash"]

    return "loaded", public_block["hash"]


def chunked(values, size):
    for index in range(0, len(values), size):
        yield values[index : index + size]


def fetch_public_blocks(args, numbers):
    block_calls = [("eth_getBlockByNumber", [hex_number(number), True]) for number in numbers]
    blocks = rpc_batch(args.public_rpc, block_calls, timeout=args.timeout)
    for number, block in zip(numbers, blocks):
        if not block:
            raise RuntimeError(f"public block {number} not found")

    tx_hashes = []
    for block in blocks:
        for tx in block["transactions"]:
            tx_hash = tx["hash"] if isinstance(tx, dict) else tx
            tx_hashes.append(tx_hash)

    raw_by_hash = {}
    for tx_chunk in chunked(tx_hashes, args.batch_size):
        calls = [("eth_getRawTransactionByHash", [tx_hash]) for tx_hash in tx_chunk]
        raw_values = rpc_batch(args.public_rpc, calls, timeout=args.timeout)
        for tx_hash, raw in zip(tx_chunk, raw_values):
            if not raw:
                raise RuntimeError(f"missing raw transaction {tx_hash}")
            raw_by_hash[tx_hash.lower()] = raw

    return blocks, raw_by_hash


def main():
    parser = argparse.ArgumentParser(description="Replay canonical IGRA blocks from public RPC into a local Engine API")
    parser.add_argument("--start", type=int, required=True)
    parser.add_argument("--end", type=int)
    parser.add_argument("--public-rpc", default="https://rpc.igralabs.com:8545")
    parser.add_argument("--local-rpc", default="http://127.0.0.1:18545")
    parser.add_argument("--auth-rpc", default="http://127.0.0.1:18551")
    parser.add_argument("--jwt-secret", required=True)
    parser.add_argument("--timeout", type=int, default=60)
    parser.add_argument("--batch-size", type=int, default=100)
    parser.add_argument(
        "--fcu-interval",
        type=int,
        default=1,
        help="Run FCU every N blocks. Use 0 to run FCU only once at the final block.",
    )
    parser.add_argument(
        "--no-check-existing",
        action="store_false",
        dest="check_existing",
        help="Do not query the local canonical block before submitting each public payload.",
    )
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args()

    latest = rpc(args.public_rpc, "eth_blockNumber", [])
    end = args.end if args.end is not None else int(latest, 16)
    total = end - args.start + 1
    last_hash = None
    global_index = 0
    for numbers in chunked(list(range(args.start, end + 1)), args.batch_size):
        blocks, raw_by_hash = fetch_public_blocks(args, numbers)
        for public_block in blocks:
            global_index += 1
            number = block_number(public_block)
            do_fcu = bool(
                not args.dry_run
                and args.fcu_interval > 0
                and (global_index % args.fcu_interval == 0 or number == end)
            )
            action, block_hash = replay_block(args, public_block, raw_by_hash, do_fcu)
            last_hash = block_hash
            if action != "skip" or global_index == 1 or global_index == total or global_index % 50 == 0:
                print(f"{number} {action} {block_hash}", flush=True)

    if not args.dry_run and args.fcu_interval == 0 and last_hash:
        forkchoice(args, last_hash, end)
        print(f"{end} forkchoice {last_hash}", flush=True)

    print(f"done start={args.start} end={end} total={total}", flush=True)


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        print(f"ERROR: {error}", file=sys.stderr)
        sys.exit(1)
