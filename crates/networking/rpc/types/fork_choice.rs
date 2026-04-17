use super::payload::PayloadStatus;
use ethrex_common::{Address, H256, serde_utils, types::Withdrawal};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForkChoiceState {
    #[allow(unused)]
    pub head_block_hash: H256,
    pub safe_block_hash: H256,
    pub finalized_block_hash: H256,
}

#[derive(Debug, Deserialize, Default, Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(unused)]
pub struct PayloadAttributesV3 {
    #[serde(
        deserialize_with = "serde_utils::u64::deser_hex_or_dec_str",
        serialize_with = "serde_utils::u64::hex_str::serialize"
    )]
    pub timestamp: u64,
    pub prev_randao: H256,
    pub suggested_fee_recipient: Address,
    pub withdrawals: Option<Vec<Withdrawal>>,
    pub parent_beacon_block_root: Option<H256>,
}

#[derive(Debug, Deserialize, Default, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
#[allow(unused)]
pub struct PayloadAttributesV4 {
    #[serde(
        deserialize_with = "serde_utils::u64::deser_hex_or_dec_str",
        serialize_with = "serde_utils::u64::hex_str::serialize"
    )]
    pub timestamp: u64,
    pub prev_randao: H256,
    pub suggested_fee_recipient: Address,
    pub withdrawals: Option<Vec<Withdrawal>>,
    pub parent_beacon_block_root: Option<H256>,
    #[serde(with = "serde_utils::u64::hex_str")]
    pub slot_number: u64,
}

#[cfg(test)]
mod tests {
    use super::{PayloadAttributesV3, PayloadAttributesV4};

    #[test]
    fn payload_attributes_v3_accept_decimal_timestamp() {
        let attrs: PayloadAttributesV3 = serde_json::from_value(serde_json::json!({
            "timestamp": "1772013518",
            "prevRandao": "0x0000000000000000000000000000000000000000000000000000000000000000",
            "suggestedFeeRecipient": "0x0000000000000000000000000000000000000fee",
            "withdrawals": [],
            "parentBeaconBlockRoot": "0x0000000000000000000000000000000000000000000000000000000000000000"
        }))
        .unwrap();

        assert_eq!(attrs.timestamp, 1_772_013_518);
    }

    #[test]
    fn payload_attributes_v4_accept_decimal_timestamp() {
        let attrs: PayloadAttributesV4 = serde_json::from_value(serde_json::json!({
            "timestamp": "1772013518",
            "prevRandao": "0x0000000000000000000000000000000000000000000000000000000000000000",
            "suggestedFeeRecipient": "0x0000000000000000000000000000000000000fee",
            "withdrawals": [],
            "parentBeaconBlockRoot": "0x0000000000000000000000000000000000000000000000000000000000000000",
            "slotNumber": "0x0"
        }))
        .unwrap();

        assert_eq!(attrs.timestamp, 1_772_013_518);
    }

    #[test]
    fn payload_attributes_preserve_hex_timestamp() {
        let attrs: PayloadAttributesV3 = serde_json::from_value(serde_json::json!({
            "timestamp": "0x699ee7ce",
            "prevRandao": "0x0000000000000000000000000000000000000000000000000000000000000000",
            "suggestedFeeRecipient": "0x0000000000000000000000000000000000000fee",
            "withdrawals": [],
            "parentBeaconBlockRoot": "0x0000000000000000000000000000000000000000000000000000000000000000"
        }))
        .unwrap();

        assert_eq!(attrs.timestamp, 0x699e_e7ce);
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForkChoiceResponse {
    pub payload_status: PayloadStatus,
    #[serde(with = "serde_utils::u64::hex_str_opt_padded")]
    pub payload_id: Option<u64>,
}

impl ForkChoiceResponse {
    pub fn set_id(&mut self, id: u64) {
        self.payload_id = Some(id)
    }
}

impl From<PayloadStatus> for ForkChoiceResponse {
    fn from(value: PayloadStatus) -> Self {
        Self {
            payload_status: value,
            payload_id: None,
        }
    }
}
