use crate::Fixed128;

use super::runtime_state::BlockTokenState;
use super::proto::Brc4;
use super::structs::Brc4ParseErr;
use nint_blk::CoinType;

fn coin() -> CoinType {
    CoinType::from(nint_blk::Bitcoin)
}

fn ct() -> &'static str {
    "application/json"
}

#[test]
fn try_parse_rejects_wrong_protocol() {
    let content = br#"{"p":"not-brc","op":"mint","tick":"ABCD","amt":"1"}"#;
    assert!(matches!(BlockTokenState::try_parse(ct(), content, 0, coin()), Err(Brc4ParseErr::WrongProtocol)));
}

#[test]
fn try_parse_rejects_invalid_utf8() {
    let content = b"\xFF\xFF";
    assert!(matches!(BlockTokenState::try_parse(ct(), content, 0, coin()), Err(Brc4ParseErr::InvalidUtf8)));
}

#[test]
fn try_parse_accepts_valid_deploy() {
    let content = br#"{"p":"brc-20","op":"deploy","tick":"abcd","max":"100","lim":"100","dec":"0"}"#;
    let parsed = BlockTokenState::try_parse(ct(), content, 0, coin()).unwrap();
    match parsed {
        Brc4::Deploy { proto } => {
            assert_eq!(format!("{}", proto.tick), "abcd");
        }
        _ => panic!("expected deploy"),
    }
}

#[test]
fn try_parse_rejects_zero_transfer() {
    let content = br#"{"p":"brc-20","op":"transfer","tick":"abcd","amt":"0"}"#;
    assert!(matches!(BlockTokenState::try_parse(ct(), content, 0, coin()), Err(Brc4ParseErr::WrongProtocol)));
}

#[test]
fn try_parse_rejects_zero_mint() {
    let content = br#"{"p":"brc-20","op":"mint","tick":"abcd","amt":"0"}"#;
    assert!(matches!(BlockTokenState::try_parse(ct(), content, 0, coin()), Err(Brc4ParseErr::WrongProtocol)));
}

#[test]
fn try_parse_accepts_self_mint_max_zero_lim_omitted() {
    // self_mint=true, max=0, lim omitted should be accepted by try_parse (normalization happens later)
    let content = br#"{"p":"brc-20","op":"deploy","tick":"abcde","max":"0","self_mint":"true"}"#;
    let parsed = BlockTokenState::try_parse(ct(), content, coin().self_mint_activation_height.unwrap_or_default() as u32, coin()).unwrap();
    match parsed {
        Brc4::Deploy { proto } => {
            assert_eq!(format!("{}", proto.tick), "abcde");
            assert!(proto.self_mint);
            assert_eq!(proto.max, Fixed128::from(u64::MAX));
            assert_eq!(proto.lim.unwrap(), proto.max);
        }
        _ => panic!("expected deploy"),
    }
}

#[test]
fn try_parse_rejects_5byte_without_self_mint() {
    // 5-byte tick without self_mint must be rejected at parse time
    let content = br#"{"p":"brc-20","op":"deploy","tick":"abcde","max":"1"}"#;
    assert!(matches!(BlockTokenState::try_parse(ct(), content, 0, coin()), Err(Brc4ParseErr::WrongProtocol)));
}

#[test]
fn try_parse_rejects_dec_over_18() {
    let content = br#"{"p":"brc-20","op":"deploy","tick":"abcd","max":"1","lim":"1","dec":"19"}"#;
    assert!(matches!(BlockTokenState::try_parse(ct(), content, 0, coin()), Err(Brc4ParseErr::WrongProtocol)));
}

#[test]
fn try_parse_rejects_numeric_not_string() {
    // Providing a numeric field as a JSON number should be rejected (must be strings)
    let content = br#"{"p":"brc-20","op":"deploy","tick":"abcd","max":100,"lim":"1","dec":"0"}"#;
    assert!(matches!(BlockTokenState::try_parse(ct(), content, 0, coin()), Err(Brc4ParseErr::WrongProtocol)));
}

#[test]
fn try_parse_rejects_six_byte_tick() {
    // 6-byte tick should be rejected at parsing
    let content = br#"{"p":"brc-20","op":"deploy","tick":"abcdef","max":"1","lim":"1","dec":"0"}"#;
    assert!(matches!(BlockTokenState::try_parse(ct(), content, 0, coin()), Err(Brc4ParseErr::WrongProtocol)));
}

#[test]
fn try_parse_accepts_valid_mint_and_transfer() {
    let mint = br#"{"p":"brc-20","op":"mint","tick":"abcd","amt":"1"}"#;
    assert!(matches!(BlockTokenState::try_parse(ct(), mint, 0, coin()), Ok(Brc4::Mint { .. })));

    let transfer = br#"{"p":"brc-20","op":"transfer","tick":"abcd","amt":"1"}"#;
    assert!(matches!(BlockTokenState::try_parse(ct(), transfer, 0, coin()), Ok(Brc4::Transfer { .. })));
}
