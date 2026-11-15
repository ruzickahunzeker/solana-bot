use crate::{application::ports::address_parser::TradeAddressParser, domain::trade::Address};
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;

pub(crate) struct SdkAddressParser;

impl TradeAddressParser for SdkAddressParser {
    fn parse(&self, value: &str) -> anyhow::Result<Address> {
        Ok(Address::from_bytes(Pubkey::from_str(value)?.to_bytes()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::trade::{SOL_MINT, USD1_MINT, USDC_MINT, WSOL_MINT};

    #[test]
    fn adapter_preserves_sdk_address_bytes_for_known_and_generated_addresses() {
        let mut values = [SOL_MINT, WSOL_MINT, USDC_MINT, USD1_MINT]
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        for seed in 0..32u8 {
            let bytes = std::array::from_fn(|index| seed.wrapping_add(index as u8));
            values.push(Pubkey::new_from_array(bytes).to_string());
        }
        for value in values {
            assert_eq!(
                SdkAddressParser.parse(&value).unwrap().to_bytes(),
                Pubkey::from_str(&value).unwrap().to_bytes()
            );
        }
        // Quote alias normalization is application policy, not parser behavior.
        assert_ne!(
            SdkAddressParser.parse(SOL_MINT).unwrap(),
            SdkAddressParser.parse(WSOL_MINT).unwrap()
        );
    }

    #[test]
    fn adapter_preserves_sdk_syntax_errors_without_trimming_or_normalization() {
        let values = [
            String::new(),
            " ".to_owned(),
            "invalid-0OIl".to_owned(),
            format!(" {WSOL_MINT}"),
            format!("{WSOL_MINT} "),
            "1".repeat(31),
            "1".repeat(33),
            "z".repeat(45),
            "\u{2603}".to_owned(),
        ];
        for value in values {
            assert_eq!(
                SdkAddressParser.parse(&value).unwrap_err().to_string(),
                Pubkey::from_str(&value).unwrap_err().to_string()
            );
        }
    }
}
