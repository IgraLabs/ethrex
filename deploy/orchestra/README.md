# IGRA Orchestra Ethrex Deployment

This directory contains the ethrex-side replacement for the official orchestra
`execution-layer` service. The intent is to keep the same service name and
network endpoints used by kaspad, rpc-provider, and node-health-check-client:

- `execution-layer:8545` for JSON-RPC
- `execution-layer:8546` for WebSocket JSON-RPC
- `execution-layer:8551` for Engine API
- `execution-layer:9001` for Prometheus metrics

## Usage

In the orchestra repository:

1. Clone this ethrex repository into `build/repos/ethrex`.
2. Copy `deploy/orchestra/docker-compose.ethrex.yml` into the orchestra root.
3. Set `ETHREX_VERSION` in the environment or `.env`.
4. Start with both compose files:

```sh
docker compose -f docker-compose.yml -f docker-compose.ethrex.yml up -d --build execution-layer kaspad node-health-check-client
```

For a full mainnet deployment, run the normal orchestra mainnet setup first so
`.env`, keys, wallet files, and network params directories are prepared. Then
start with the ethrex override file.

The override intentionally keeps the service name `execution-layer`, so the rest
of the stack does not need endpoint changes.
