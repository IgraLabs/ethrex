# Ethrex + Orchestra Brief

This note is for DevOps users who need to understand what we added for IGRA in `ethrex`, what we added in `igra-orchestra`, and how to use orchestra to run the stack.

Last updated: 2026-04-28

## Scope

This covers the current integration branches:

- `ethrex`: `igra-mainnet-ethrex-integration`
- `igra-orchestra`: `ethrex-integration`

It focuses on the L1 / execution-layer side, not the L2 prover stack.

When this document links to repository files, it uses branch-specific GitHub
URLs so readers see the exact integration branch content rather than whatever is
currently on `main`.

## Repositories

### `ethrex`

Role:

- Ethereum-compatible execution layer
- Runs the IGRA execution client in place of the default `reth` execution layer
- Exposes the same interfaces expected by orchestra and `kaspad`

Relevant paths:

- [`igra/run-igra-el.sh`](https://github.com/IgraLabs/ethrex/blob/igra-mainnet-ethrex-integration/igra/run-igra-el.sh)
- [`igra/genesis.template.json`](https://github.com/IgraLabs/ethrex/blob/igra-mainnet-ethrex-integration/igra/genesis.template.json)
- [`igra/network-params.template.md`](https://github.com/IgraLabs/ethrex/blob/igra-mainnet-ethrex-integration/igra/network-params.template.md)
- [`deploy/orchestra/README.md`](https://github.com/IgraLabs/ethrex/blob/igra-mainnet-ethrex-integration/deploy/orchestra/README.md)
- [`deploy/orchestra/docker-compose.ethrex.yml`](https://github.com/IgraLabs/ethrex/blob/igra-mainnet-ethrex-integration/deploy/orchestra/docker-compose.ethrex.yml)

### `igra-orchestra`

Role:

- Docker Compose deployment for the full IGRA node stack
- Starts `kaspad`, the execution layer, worker pairs (`rpc-provider-*` + `kaswallet-*`), Traefik, and health monitoring

Relevant paths:

- [`docker-compose.yml`](https://github.com/IgraLabs/igra-orchestra/blob/ethrex-integration/docker-compose.yml)
- [`README.md`](https://github.com/IgraLabs/igra-orchestra/blob/ethrex-integration/README.md)
- [`doc/quick-setup-mainnet.md`](https://github.com/IgraLabs/igra-orchestra/blob/ethrex-integration/doc/quick-setup-mainnet.md)
- [`deploy/ethrex-mainnet/README.md`](https://github.com/IgraLabs/igra-orchestra/blob/ethrex-integration/deploy/ethrex-mainnet/README.md)
- [`deploy/ethrex-galleon-staging/README.md`](https://github.com/IgraLabs/igra-orchestra/blob/ethrex-integration/deploy/ethrex-galleon-staging/README.md)

## What We Added In `ethrex`

The main goal was to make `ethrex` look like a drop-in replacement for the orchestra `execution-layer` service.

### 1. IGRA deployment packaging

We added:

- a runtime entrypoint script
- a genesis template
- a network-parameters template
- an orchestra override file

This came in the packaging commit series around:

- `22c821f98 Add IGRA ethrex deployment packaging`
- `ba2a077e4 Add deposit contract address to IGRA genesis template`

### 2. Runtime generation of IGRA-specific genesis

[`igra/run-igra-el.sh`](https://github.com/IgraLabs/ethrex/blob/igra-mainnet-ethrex-integration/igra/run-igra-el.sh) takes the same environment values orchestra already has for IGRA and generates:

- `genesis.json`
- `network-params.md`
- the genesis `extraData` hash from `network-params.md`

That keeps the execution-layer startup aligned with the network parameters used by `kaspad`.

### 3. Orchestra-compatible endpoints

The ethrex service is started with the endpoints orchestra expects:

- HTTP JSON-RPC on `8545`
- WebSocket JSON-RPC on `8546`
- Engine API on `8551`
- metrics on `9001`

See [`igra/run-igra-el.sh`](https://github.com/IgraLabs/ethrex/blob/igra-mainnet-ethrex-integration/igra/run-igra-el.sh).

### 4. IGRA compatibility fixes in ethrex itself

The integration branch also includes runtime fixes so ethrex behaves correctly under the IGRA/Kaspad engine flow:

- payload-building stabilization
- engine replay handling for IGRA mainnet flow
- acceptance of decimal payload timestamps
- Prague request-contract handling when the deposit predeploy is absent
- improved RPC request error handling

Representative commits:

- `91e0dbef9 Add Igra execution client compatibility hooks`
- `8f9c8bfa2 Accept decimal payload attribute timestamps`
- `c16a84484 Handle IGRA mainnet engine replay`
- `f12b903ba Stabilize IGRA payload building`
- `9b587f75e fix rpc request error handling`

## What We Added In `igra-orchestra`

The orchestra base stack still defaults to `reth` as the `execution-layer`; see [`docker-compose.yml`](https://github.com/IgraLabs/igra-orchestra/blob/ethrex-integration/docker-compose.yml).

Our integration work adds an ethrex-based path without changing the overall orchestra model.

### 1. Ethrex integration branch

Branch:

- `ethrex-integration`

Key commits:

- `f25ab38 Add ethrex galleon staging deployment`
- `b070613 Set Galleon reference hashes`
- `557f71b Fix Galleon health client healthcheck`

### 2. Mainnet deployment override

We added a dedicated deploy directory:

- [`deploy/ethrex-mainnet`](https://github.com/IgraLabs/igra-orchestra/tree/ethrex-integration/deploy/ethrex-mainnet)

It contains:

- `.env.example`
- `docker-compose.ethrex-mainnet.yml`
- a deployment README

Purpose:

- run the normal orchestra mainnet stack with `ethrex` in place of `reth`
- keep the standard service names, ports, and worker model
- make the mainnet ethrex path reproducible from orchestra itself instead of relying only on the ethrex repo-side override

### 3. Galleon-specific deployment override

We added a dedicated deploy directory:

- [`deploy/ethrex-galleon-staging`](https://github.com/IgraLabs/igra-orchestra/tree/ethrex-integration/deploy/ethrex-galleon-staging)

It contains:

- `.env.example`
- `docker-compose.ethrex-galleon.yml`
- a deployment README

Purpose:

- run Galleon on the same machine as another orchestra deployment
- replace `reth` with `ethrex`
- keep fixed service names inside Docker (`execution-layer`, `kaspad`, etc.)
- isolate host ports and container names with the `galleon-` prefix

### 4. Health-client compatibility

The override also sets health-client metadata so the monitoring service reports the ethrex image version correctly. See [`deploy/ethrex-galleon-staging/docker-compose.ethrex-galleon.yml`](https://github.com/IgraLabs/igra-orchestra/blob/ethrex-integration/deploy/ethrex-galleon-staging/docker-compose.ethrex-galleon.yml).

## What Was Ported vs Changed

### Ported from original orchestra assumptions

These assumptions were kept:

- `kaspad` talks to `http://execution-layer:8545`
- node health check talks to `http://execution-layer:8545`
- JWT auth is mounted into the execution layer
- network parameters are mounted from orchestra into the execution layer
- compose owns service lifecycle
- frontend still means `rpc-provider-*` + `kaswallet-*` worker pairs behind Traefik

### Changed for ethrex

- execution layer implementation changed from `reth` to `ethrex`
- ethrex builds its IGRA genesis at startup from orchestra env
- ethrex runs with `--p2p.disabled`
- builder ordering is set to `fifo`
- Galleon deployment uses separate host ports and prefixed container names

## How To Think About Orchestra

At a high level, orchestra is just profile-based Docker Compose around a fixed service graph.

### Backend profile

Starts the core node:

- `kaspad`
- `execution-layer`
- `node-health-check-client`

### Frontend profiles

Start worker pairs:

- `rpc-provider-N`
- `kaswallet-N`

Available profiles:

- `frontend-w1` through `frontend-w20`

Example reference: [`doc/node-operations/worker-configuration.md`](https://github.com/IgraLabs/igra-orchestra/blob/ethrex-integration/doc/node-operations/worker-configuration.md)

### Usual startup order

1. Prepare `.env`
2. Create `keys/jwt.hex`
3. Start `backend`
4. Wait for sync / health
5. Start the desired `frontend-wN` profile
6. Sync wallet addresses into `.env`
7. Restart frontend if wallet addresses changed

## How To Use Orchestra With Ethrex

### Mainnet

Current state:

- orchestra still ships the normal `reth` mainnet path by default
- the `ethrex-integration` branch now also contains a dedicated mainnet ethrex deployment package
- the generic override still exists on the `ethrex` repo side, but the preferred operator entry point is now the orchestra package

Preferred orchestra-side package:

- [`deploy/ethrex-mainnet/README.md`](https://github.com/IgraLabs/igra-orchestra/blob/ethrex-integration/deploy/ethrex-mainnet/README.md)
- [`deploy/ethrex-mainnet/docker-compose.ethrex-mainnet.yml`](https://github.com/IgraLabs/igra-orchestra/blob/ethrex-integration/deploy/ethrex-mainnet/docker-compose.ethrex-mainnet.yml)

Ethrex-side packaging that this uses:

- [`deploy/orchestra/README.md`](https://github.com/IgraLabs/ethrex/blob/igra-mainnet-ethrex-integration/deploy/orchestra/README.md)
- [`deploy/orchestra/docker-compose.ethrex.yml`](https://github.com/IgraLabs/ethrex/blob/igra-mainnet-ethrex-integration/deploy/orchestra/docker-compose.ethrex.yml)

Operationally, the model is:

1. run normal orchestra mainnet preparation
2. clone ethrex into `build/repos/ethrex`
3. create `.env` from `deploy/ethrex-mainnet/.env.example`
4. append `versions.mainnet.env`
5. start orchestra with `docker-compose.yml` plus `deploy/ethrex-mainnet/docker-compose.ethrex-mainnet.yml`

### Galleon testnet

This is the repo-backed deployment path we actually formalized in orchestra.

Use:

- [`deploy/ethrex-galleon-staging/README.md`](https://github.com/IgraLabs/igra-orchestra/blob/ethrex-integration/deploy/ethrex-galleon-staging/README.md)

Summary flow:

1. checkout `igra-orchestra` branch `ethrex-integration`
2. create `.env` from `deploy/ethrex-galleon-staging/.env.example`
3. append `versions.testnet.env`
4. create `keys/jwt.hex`
5. clone `ethrex` branch `igra-mainnet-ethrex-integration` into `build/repos/ethrex`
6. validate compose config
7. build `execution-layer`
8. start `--profile backend`
9. after sync, start `--profile frontend-wN`

Important host ports for the Galleon ethrex staging override:

- ethrex HTTP RPC: `127.0.0.1:19545`
- ethrex WS RPC: `127.0.0.1:19546`
- Traefik RPC, if frontend is enabled: `127.0.0.1:18545`

Expected chain ID:

- Galleon: `0x97b4`

## Operational Notes

- The orchestra worker model did not change for ethrex. Load still comes through the usual `rpc-provider` + `kaswallet` pairs.
- `W{N}_WALLET_TO_ADDRESS` remains critical; it is the change return address for KasWallet.
- Backend and frontend should still be treated as separate lifecycle groups.
- For same-host parallel deployments, use separate directories and separate host ports.

## Recommended References

- Ethrex packaging: [`deploy/orchestra/README.md`](https://github.com/IgraLabs/ethrex/blob/igra-mainnet-ethrex-integration/deploy/orchestra/README.md)
- Mainnet override: [`deploy/ethrex-mainnet/README.md`](https://github.com/IgraLabs/igra-orchestra/blob/ethrex-integration/deploy/ethrex-mainnet/README.md)
- Galleon override: [`deploy/ethrex-galleon-staging/README.md`](https://github.com/IgraLabs/igra-orchestra/blob/ethrex-integration/deploy/ethrex-galleon-staging/README.md)
- Orchestra mainnet quick start: [`doc/quick-setup-mainnet.md`](https://github.com/IgraLabs/igra-orchestra/blob/ethrex-integration/doc/quick-setup-mainnet.md)
- Orchestra worker model: [`doc/node-operations/worker-configuration.md`](https://github.com/IgraLabs/igra-orchestra/blob/ethrex-integration/doc/node-operations/worker-configuration.md)
- Wallet address sync: [`doc/node-operations/wallet-management.md`](https://github.com/IgraLabs/igra-orchestra/blob/ethrex-integration/doc/node-operations/wallet-management.md)
