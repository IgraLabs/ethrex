# Galleon Staging Stress Test Runbook

This runbook describes how to stress-test the Galleon testnet execution layer
deployed on the staging host.

Last verified: 2026-04-22

## Scope

Use this only for the Galleon testnet deployment on staging.

Do not use the mainnet RPC port for these tests.

| Deployment | HTTP RPC | WS RPC | Chain ID |
| --- | --- | --- | --- |
| Galleon testnet | `127.0.0.1:19545` | `127.0.0.1:19546` | `0x97b4` |
| Current mainnet staging deployment | `127.0.0.1:18545` | N/A | `0x97b1` |

The Galleon RPC ports are bound to localhost on the staging server. Testers
should either run commands on staging or create an SSH tunnel from their laptop.

## Connect From Laptop

Create the tunnel:

```bash
ssh -N \
  -L 19545:127.0.0.1:19545 \
  -L 19546:127.0.0.1:19546 \
  roman@stage-roman.igralabs.com
```

In another terminal:

```bash
export RPC_URL=http://127.0.0.1:19545
export WS_URL=ws://127.0.0.1:19546
```

## Pre-Test Sanity Checks

Run these before every test session:

```bash
curl -sS "$RPC_URL" \
  -H 'Content-Type: application/json' \
  --data '{"jsonrpc":"2.0","method":"eth_chainId","params":[],"id":1}'

curl -sS "$RPC_URL" \
  -H 'Content-Type: application/json' \
  --data '{"jsonrpc":"2.0","method":"eth_syncing","params":[],"id":2}'

curl -sS "$RPC_URL" \
  -H 'Content-Type: application/json' \
  --data '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":3}'
```

Expected results:

- `eth_chainId` returns `0x97b4`.
- `eth_syncing` returns `false`.
- `eth_blockNumber` returns a recent block number.

If `eth_chainId` returns `0x97b1`, stop: you are connected to the mainnet
staging deployment, not Galleon.

## Read-Only RPC Stress Tests

Read-only tests are the safest first step. They do not submit transactions or
mutate chain state.

### `eth_blockNumber`

```bash
export BODY='{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}'

seq 1 10000 | xargs -n1 -P 64 sh -c '
  curl -sS -o /dev/null -w "%{http_code} %{time_total}\n" \
    "$RPC_URL" \
    -H "Content-Type: application/json" \
    --data "$BODY"
'
```

### `eth_getBlockByNumber`

```bash
export BODY='{"jsonrpc":"2.0","method":"eth_getBlockByNumber","params":["latest",false],"id":1}'

seq 1 5000 | xargs -n1 -P 64 sh -c '
  curl -sS -o /dev/null -w "%{http_code} %{time_total}\n" \
    "$RPC_URL" \
    -H "Content-Type: application/json" \
    --data "$BODY"
'
```

### Suggested Read-Only Ramp

Increase concurrency gradually:

| Duration | `xargs -P` concurrency | Notes |
| --- | ---: | --- |
| 1 minute | 16 | Smoke test |
| 3 minutes | 32 | Light load |
| 5 minutes | 64 | Moderate load |
| 5 minutes | 128 | High load |

Stop if HTTP errors appear, latency grows sharply, or the health client stops
reporting consensus health.

## Transaction Load Tests

The ethrex repo includes a transaction load-test tool at:

```text
tooling/load_test
```

Run commands from the repository root.

Before running load tests, increase the file descriptor limit in the current
shell:

```bash
ulimit -n 65536
```

### Funding Requirements

Prepare a private key file with one funded Galleon test account per line:

```text
0xabc...
0xdef...
0x123...
```

Rules:

- Use dedicated test keys only.
- Do not use production keys.
- Do not run multiple generators with the same key file at the same time. They
  will race nonces.
- Total submitted transaction count is `number_of_keys * -N`.

Example: 20 keys with `-N 50` sends 1000 transactions.

### Native Transfer Test

Start with native transfers. This path only requires the accounts in the key
file to be funded.

Small smoke:

```bash
cargo run --release --manifest-path ./tooling/load_test/Cargo.toml -- \
  -n "$RPC_URL" \
  -k ./galleon-funded-private-keys.txt \
  -t eth-transfers \
  -N 5 \
  -w 10
```

Moderate:

```bash
cargo run --release --manifest-path ./tooling/load_test/Cargo.toml -- \
  -n "$RPC_URL" \
  -k ./galleon-funded-private-keys.txt \
  -t eth-transfers \
  -N 50 \
  -w 20
```

Heavier:

```bash
cargo run --release --manifest-path ./tooling/load_test/Cargo.toml -- \
  -n "$RPC_URL" \
  -k ./galleon-funded-private-keys.txt \
  -t eth-transfers \
  -N 250 \
  -w 30
```

### Soak Test

Use `--endless` for a longer run:

```bash
cargo run --release --manifest-path ./tooling/load_test/Cargo.toml -- \
  -n "$RPC_URL" \
  -k ./galleon-funded-private-keys.txt \
  -t eth-transfers \
  -N 25 \
  -w 20 \
  --endless
```

Stop with `Ctrl-C`.

### Contract-Based Test Types

The load-test tool also supports:

```bash
-t erc20
-t fibonacci
-t io-heavy
```

These tests deploy contracts before generating load. The current tool uses this
hardcoded deployer address for deployment:

```text
0x8943545177806ED17B9F23F0a21ee5948eCaa776
```

That address must also have Galleon funds, otherwise contract-based tests will
fail during deployment.

Suggested order:

1. `eth-transfers`
2. `erc20`
3. `fibonacci`
4. `io-heavy`

`fibonacci` is CPU-heavy. `io-heavy` is storage-heavy.

## Monitoring During Tests

Open a staging shell:

```bash
ssh roman@stage-roman.igralabs.com
```

Watch container resource usage:

```bash
watch -n 5 'docker stats --no-stream galleon-kaspad galleon-execution-layer galleon-node-health-check-client'
```

Watch consensus health:

```bash
docker logs -f --tail 100 galleon-node-health-check-client
```

Healthy output should look like:

```text
Block ... in consensus (100.0% network agreement), status: healthy
```

Watch execution-layer logs:

```bash
docker logs -f --tail 100 galleon-execution-layer
```

Run a quick RPC check from staging:

```bash
curl -sS http://127.0.0.1:19545 \
  -H 'Content-Type: application/json' \
  --data '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}'
```

## Stop Criteria

Stop the active load generator if any of these happen:

- `galleon-node-health-check-client` stops reporting `status: healthy`.
- `galleon-execution-layer` becomes unhealthy or restarts.
- RPC starts returning many non-`200` HTTP responses.
- `eth_blockNumber` stops advancing for several minutes during transaction load.
- Memory approaches host limits.
- Mainnet staging containers become unhealthy.

The staging host has enough memory for normal operation, but keep watching
`galleon-kaspad` and `galleon-execution-layer` under heavy load.

## Recommended Team Test Plan

1. Run pre-test sanity checks.
2. Run read-only RPC load at concurrency `16`, then `32`, then `64`.
3. Run `eth-transfers` with 10-20 funded keys and `-N 5`.
4. If healthy, run `eth-transfers` with `-N 50`.
5. If still healthy, run a 30-60 minute `--endless` soak with `-N 25`.
6. After native transfers are stable, test `erc20`, then `fibonacci`, then
   `io-heavy`.

Record for each run:

- Start and end time.
- Test type.
- Number of keys.
- `-N` value.
- Any HTTP errors.
- Approximate latency range.
- Container CPU and memory.
- Health-client status.
