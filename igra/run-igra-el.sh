#!/bin/sh

set -e

# Genesis template path is overridable via GENESIS_TEMPLATE (e.g. the KYC zone points it at
# genesis.kyc.template.json, which adds config.kycRegistry). Defaults to the canonical template.
readonly DEFAULT_GENESIS_TEMPLATE="${GENESIS_TEMPLATE:-/app/genesis.template.json}"
readonly DEFAULT_GENESIS_JSON="/app/genesis.json"
readonly DEFAULT_JWT_FILE="/app/jwt.hex"
readonly DEFAULT_BASE_FEE="0x1"
readonly DEFAULT_DATA_DIR="/app/data"
readonly DEFAULT_NETWORK_PARAMS_TEMPLATE="/app/network-params.template.md"
readonly DEFAULT_NETWORK_PARAMS_OUTPUT="/app/network-params.md"

print_separator() {
    echo "========================================="
}

fatal_error() {
    local message="$1"
    local exit_code="${2:-1}"
    echo "FATAL: $message" >&2
    exit "$exit_code"
}

display_env_vars() {
    print_separator
    echo "ENVIRONMENT VARIABLES:"
    print_separator
    echo "Required variables:"
    echo "  CHAIN_ID: ${CHAIN_ID}"
    echo "  ONE_TIME_ADDRESS: ${ONE_TIME_ADDRESS}"
    echo "  IGRA_LAUNCH_DAA_SCORE: ${IGRA_LAUNCH_DAA_SCORE}"
    echo "  L1_REFERENCE_DAA_SCORE: ${L1_REFERENCE_DAA_SCORE}"
    echo "  L1_REFERENCE_TIMESTAMP: ${L1_REFERENCE_TIMESTAMP}"
    echo "  TX_ID_PREFIX: ${TX_ID_PREFIX}"
    echo "  MIN_PROTOCOL_FEE_PER_GAS_GWEI: ${MIN_PROTOCOL_FEE_PER_GAS_GWEI}"
    echo "  IGRA_LOCK_SCRIPT_PUBKEY: ${IGRA_LOCK_SCRIPT_PUBKEY}"
    echo "  IGRA_ENTRY_MIN_AMOUNT: ${IGRA_ENTRY_MIN_AMOUNT}"
    echo "  BITCOIN_BLOCK_HASH: ${BITCOIN_BLOCK_HASH}"
    echo "  ETHEREUM_BLOCK_HASH: ${ETHEREUM_BLOCK_HASH}"
    echo "  KASPA_BLOCK_HASH: ${KASPA_BLOCK_HASH}"
    echo ""
    echo "Optional variables:"
    echo "  GENESIS_TEMPLATE_JSON: ${GENESIS_TEMPLATE_JSON:-$DEFAULT_GENESIS_TEMPLATE}"
    echo "  GENESIS_JSON: ${GENESIS_JSON:-$DEFAULT_GENESIS_JSON}"
    echo "  JWT_FILE: ${JWT_FILE:-$DEFAULT_JWT_FILE}"
    echo "  BASE_FEE_PER_GAS: ${BASE_FEE_PER_GAS:-$DEFAULT_BASE_FEE}"
    echo "  DATA_DIR: ${DATA_DIR:-$DEFAULT_DATA_DIR}"
    echo "  NETWORK_PARAMS_TEMPLATE: ${NETWORK_PARAMS_TEMPLATE:-$DEFAULT_NETWORK_PARAMS_TEMPLATE}"
    echo "  NETWORK_PARAMS_OUTPUT: ${NETWORK_PARAMS_OUTPUT:-$DEFAULT_NETWORK_PARAMS_OUTPUT}"
    print_separator
    echo ""
}

validate_required_vars() {
    local var_name
    local var_value

    for var_name in CHAIN_ID ONE_TIME_ADDRESS IGRA_LAUNCH_DAA_SCORE L1_REFERENCE_DAA_SCORE L1_REFERENCE_TIMESTAMP TX_ID_PREFIX MIN_PROTOCOL_FEE_PER_GAS_GWEI IGRA_LOCK_SCRIPT_PUBKEY IGRA_ENTRY_MIN_AMOUNT BITCOIN_BLOCK_HASH ETHEREUM_BLOCK_HASH KASPA_BLOCK_HASH; do
        eval "var_value=\${$var_name}"
        if [ -z "$var_value" ]; then
            fatal_error "$var_name environment variable is not set" 4
        fi
    done
}

validate_file_exists() {
    local file_path="$1"
    local file_description="$2"
    local exit_code="${3:-1}"

    if [ ! -f "$file_path" ]; then
        fatal_error "$file_description not found at $file_path" "$exit_code"
    fi
}

calculate_genesis_timestamp() {
    GENESIS_TIMESTAMP=$(((IGRA_LAUNCH_DAA_SCORE/10) - (L1_REFERENCE_DAA_SCORE/10) + L1_REFERENCE_TIMESTAMP - 1))
    export GENESIS_TIMESTAMP

    echo "Calculated values:"
    echo "  GENESIS_TIMESTAMP: ${GENESIS_TIMESTAMP}"
    echo ""
}

generate_configurator_getter_code() {
    local addr_no_prefix
    addr_no_prefix=$(echo "$ONE_TIME_ADDRESS" | sed 's/^0x//' | tr 'A-Z' 'a-z')

    CONFIGURATOR_GETTER_CODE="0x73${addr_no_prefix}5f5260205ff3"
    export CONFIGURATOR_GETTER_CODE

    echo "  CONFIGURATOR_GETTER_CODE: ${CONFIGURATOR_GETTER_CODE}"
}

generate_ts_reference_getter_code() {
    local daa_hex
    local timestamp_hex

    daa_hex=$(printf "%016x" "$L1_REFERENCE_DAA_SCORE")
    timestamp_hex=$(printf "%016x" "$L1_REFERENCE_TIMESTAMP")

    TS_REFERENCE_GETTER_CODE="0x67${daa_hex}5f5267${timestamp_hex}60205260405ff3"
    export TS_REFERENCE_GETTER_CODE

    echo "  TS_REFERENCE_GETTER_CODE: ${TS_REFERENCE_GETTER_CODE}"
    echo ""
}

replace_network_params_vars() {
    local input_file="$1"
    local output_file="$2"

    sed \
        -e "s|\${CHAIN_ID}|${CHAIN_ID}|g" \
        -e "s|\${IGRA_LAUNCH_DAA_SCORE}|${IGRA_LAUNCH_DAA_SCORE}|g" \
        -e "s|\${TX_ID_PREFIX}|${TX_ID_PREFIX}|g" \
        -e "s|\${L1_REFERENCE_TIMESTAMP}|${L1_REFERENCE_TIMESTAMP}|g" \
        -e "s|\${L1_REFERENCE_DAA_SCORE}|${L1_REFERENCE_DAA_SCORE}|g" \
        -e "s|\${MIN_PROTOCOL_FEE_PER_GAS_GWEI}|${MIN_PROTOCOL_FEE_PER_GAS_GWEI}|g" \
        -e "s|\${IGRA_LOCK_SCRIPT_PUBKEY}|${IGRA_LOCK_SCRIPT_PUBKEY}|g" \
        -e "s|\${IGRA_ENTRY_MIN_AMOUNT}|${IGRA_ENTRY_MIN_AMOUNT}|g" \
        -e "s|\${BITCOIN_BLOCK_HASH}|${BITCOIN_BLOCK_HASH}|g" \
        -e "s|\${ETHEREUM_BLOCK_HASH}|${ETHEREUM_BLOCK_HASH}|g" \
        -e "s|\${KASPA_BLOCK_HASH}|${KASPA_BLOCK_HASH}|g" \
        "$input_file" > "$output_file"
}

generate_network_params_hash() {
    local template="$1"
    local output="$2"
    local hash

    echo "Generating network params hash..."

    mkdir -p "$(dirname "$output")"
    replace_network_params_vars "$template" "$output" || {
        fatal_error "Failed to generate network params from template" 5
    }

    if command -v sha256sum > /dev/null 2>&1; then
        hash=$(sha256sum "$output" | awk '{print $1}')
    elif command -v shasum > /dev/null 2>&1; then
        hash=$(shasum -a 256 "$output" | awk '{print $1}')
    else
        fatal_error "Neither sha256sum nor shasum found" 6
    fi

    if [ ${#hash} -ne 64 ]; then
        fatal_error "SHA256 hash has unexpected length ${#hash}: '${hash}'" 6
    fi

    EXTRA_DATA="0x${hash}"
    export EXTRA_DATA

    EXTRA_DATA_SHORT="0x$(echo "$hash" | cut -c1-14)..$(echo "$hash" | cut -c51-64)"
    export EXTRA_DATA_SHORT

    echo "  EXTRA_DATA: ${EXTRA_DATA} (SHA256 of network params)"
    echo "  EXTRA_DATA_SHORT: ${EXTRA_DATA_SHORT} (32-byte CLI form)"
    echo ""
}

replace_env_vars() {
    local input_file="$1"
    local output_file="$2"

    sed \
        -e "s|\${CHAIN_ID}|${CHAIN_ID}|g" \
        -e "s|\${GENESIS_TIMESTAMP}|${GENESIS_TIMESTAMP}|g" \
        -e "s|\${BASE_FEE_PER_GAS}|${BASE_FEE_PER_GAS}|g" \
        -e "s|\${CONFIGURATOR_GETTER_CODE}|${CONFIGURATOR_GETTER_CODE}|g" \
        -e "s|\${TS_REFERENCE_GETTER_CODE}|${TS_REFERENCE_GETTER_CODE}|g" \
        -e "s|\${EXTRA_DATA}|${EXTRA_DATA}|g" \
        "$input_file" > "$output_file"
}

generate_genesis_json() {
    local template="$1"
    local output="$2"
    local temp_file="${output}.tmp"

    echo "Generating genesis.json:"
    echo "  Chain ID: ${CHAIN_ID}"
    echo "  Genesis Timestamp: ${GENESIS_TIMESTAMP}"
    echo "  One-time Address: ${ONE_TIME_ADDRESS}"
    echo "  Base Fee Per Gas: ${BASE_FEE_PER_GAS}"
    print_separator

    replace_env_vars "$template" "$temp_file" || {
        fatal_error "Failed to generate genesis.json from template" 5
    }

    sed 's/"chainId": "\([0-9]*\)"/"chainId": \1/' "$temp_file" > "$output" || {
        rm -f "$temp_file"
        fatal_error "Failed to fix chainId in genesis.json" 5
    }

    rm -f "$temp_file"
    echo "Successfully generated genesis.json"
    echo ""
}

display_genesis_content() {
    local genesis_file="$1"

    echo "Generated genesis.json content:"
    print_separator
    cat "$genesis_file"
    echo ""
    print_separator
    echo ""
}

extract_genesis_value() {
    local genesis_file="$1"
    local key="$2"

    grep -o "\"$key\": *\"[^\"]*\"" "$genesis_file" |
    awk -F':' '{gsub(/"| /, "", $2); print $2}'
}

extract_genesis_params() {
    local genesis_file="$1"

    GENESIS_gasLimit=$(extract_genesis_value "$genesis_file" "gasLimit")

    if [ -n "${GENESIS_gasLimit}" ]; then
        GENESIS_gasLimit=$(printf "%d\n" "${GENESIS_gasLimit}")
    fi

    echo "Extracted from genesis.json:"
    echo "  GENESIS_gasLimit: ${GENESIS_gasLimit}"
    print_separator
    echo ""
}

display_ethrex_params() {
    echo "Starting ethrex node with parameters:"
    print_separator
    echo "  Network genesis: ${genesis_json}"
    echo "  JWT Secret: ${jwt_file}"
    echo "  HTTP Address: 0.0.0.0:8545"
    echo "  WebSocket Address: 0.0.0.0:8546"
    echo "  Auth RPC Address: 0.0.0.0:8551"
    echo "  Data Directory: ${data_dir}"
    echo "  Builder Extra Data: '${EXTRA_DATA_SHORT}' (full hash: ${EXTRA_DATA})"
    echo "  Builder Gas Limit: ${GENESIS_gasLimit}"
    echo "  Builder TX Ordering: fifo"
    echo "  Metrics Address: 0.0.0.0:9001"
    echo "  P2P: disabled"
    echo "  Sync mode: full"
    print_separator
    echo ""
}

start_ethrex_node() {
    mkdir -p "${data_dir}"
    exec ethrex \
        --network "${genesis_json}" \
        --datadir "${data_dir}" \
        --http.addr 0.0.0.0 \
        --http.port 8545 \
        --ws.enabled \
        --ws.addr 0.0.0.0 \
        --ws.port 8546 \
        --authrpc.addr 0.0.0.0 \
        --authrpc.port 8551 \
        --authrpc.jwtsecret "${jwt_file}" \
        --metrics \
        --metrics.addr 0.0.0.0 \
        --metrics.port 9001 \
        --p2p.disabled \
        --builder.extra-data "${EXTRA_DATA_SHORT}" \
        --builder.gas-limit "${GENESIS_gasLimit}" \
        --builder.tx-ordering fifo \
        --syncmode full
}

display_env_vars

genesis_template="${GENESIS_TEMPLATE_JSON:-$DEFAULT_GENESIS_TEMPLATE}"
genesis_json="${GENESIS_JSON:-$DEFAULT_GENESIS_JSON}"
jwt_file="${JWT_FILE:-$DEFAULT_JWT_FILE}"
data_dir="${DATA_DIR:-$DEFAULT_DATA_DIR}"
network_params_template="${NETWORK_PARAMS_TEMPLATE:-$DEFAULT_NETWORK_PARAMS_TEMPLATE}"
network_params_output="${NETWORK_PARAMS_OUTPUT:-$DEFAULT_NETWORK_PARAMS_OUTPUT}"

BASE_FEE_PER_GAS="${BASE_FEE_PER_GAS:-$DEFAULT_BASE_FEE}"
export BASE_FEE_PER_GAS

validate_required_vars
validate_file_exists "$genesis_template" "genesis.template.json" 1
validate_file_exists "$network_params_template" "network-params.template.md" 1

calculate_genesis_timestamp
generate_configurator_getter_code
generate_ts_reference_getter_code

export CHAIN_ID
export TX_ID_PREFIX
export MIN_PROTOCOL_FEE_PER_GAS_GWEI
export IGRA_LOCK_SCRIPT_PUBKEY
export IGRA_ENTRY_MIN_AMOUNT
export BITCOIN_BLOCK_HASH
export ETHEREUM_BLOCK_HASH
export KASPA_BLOCK_HASH

generate_network_params_hash "$network_params_template" "$network_params_output"
generate_genesis_json "$genesis_template" "$genesis_json"

validate_file_exists "$genesis_json" "genesis.json" 1
display_genesis_content "$genesis_json"

extract_genesis_params "$genesis_json"

validate_file_exists "$jwt_file" "jwt.hex" 2

display_ethrex_params
start_ethrex_node
