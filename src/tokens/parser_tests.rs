use super::parser::TokenCache;
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
    assert!(matches!(TokenCache::try_parse(ct(), content, coin()), Err(Brc4ParseErr::WrongProtocol)));
}

#[test]
fn try_parse_rejects_invalid_utf8() {
    let content = b"\xFF\xFF";
    assert!(matches!(TokenCache::try_parse(ct(), content, coin()), Err(Brc4ParseErr::InvalidUtf8)));
}

#[test]
fn try_parse_accepts_valid_deploy() {
    let content = br#"{"p":"brc-20","op":"deploy","tick":"abcd","max":"100","lim":"100","dec":"0"}"#;
    let parsed = TokenCache::try_parse(ct(), content, coin()).unwrap();
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
    assert!(matches!(TokenCache::try_parse(ct(), content, coin()), Err(Brc4ParseErr::WrongProtocol)));
}

#[test]
fn try_parse_rejects_zero_mint() {
    let content = br#"{"p":"brc-20","op":"mint","tick":"abcd","amt":"0"}"#;
    assert!(matches!(TokenCache::try_parse(ct(), content, coin()), Err(Brc4ParseErr::WrongProtocol)));
}

#[test]
fn try_parse_accepts_self_mint_max_zero_lim_omitted() {
    // self_mint=true, max=0, lim omitted should be accepted by try_parse (normalization happens later)
    let content = br#"{"p":"brc-20","op":"deploy","tick":"abcde","max":"0","self_mint":"true"}"#;
    let parsed = TokenCache::try_parse(ct(), content, coin()).unwrap();
    match parsed {
        Brc4::Deploy { proto } => {
            assert_eq!(format!("{}", proto.tick), "abcde");
            assert!(proto.self_mint);
            assert_eq!(proto.max.to_string(), "0");
            assert!(proto.lim.is_none());
        }
        _ => panic!("expected deploy"),
    }
}

#[test]
fn try_parse_rejects_5byte_without_self_mint() {
    // 5-byte tick without self_mint must be rejected at parse time
    let content = br#"{"p":"brc-20","op":"deploy","tick":"abcde","max":"1"}"#;
    assert!(matches!(TokenCache::try_parse(ct(), content, coin()), Err(Brc4ParseErr::WrongProtocol)));
}

#[test]
fn try_parse_rejects_dec_over_18() {
    let content = br#"{"p":"brc-20","op":"deploy","tick":"abcd","max":"1","lim":"1","dec":"19"}"#;
    assert!(matches!(TokenCache::try_parse(ct(), content, coin()), Err(Brc4ParseErr::WrongProtocol)));
}

#[test]
fn try_parse_rejects_numeric_not_string() {
    // Providing a numeric field as a JSON number should be rejected (must be strings)
    let content = br#"{"p":"brc-20","op":"deploy","tick":"abcd","max":100,"lim":"1","dec":"0"}"#;
    assert!(matches!(TokenCache::try_parse(ct(), content, coin()), Err(Brc4ParseErr::WrongProtocol)));
}

#[test]
fn try_parse_rejects_six_byte_tick() {
    // 6-byte tick should be rejected at parsing
    let content = br#"{"p":"brc-20","op":"deploy","tick":"abcdef","max":"1","lim":"1","dec":"0"}"#;
    assert!(matches!(TokenCache::try_parse(ct(), content, coin()), Err(Brc4ParseErr::WrongProtocol)));
}

#[test]
fn try_parse_accepts_valid_mint_and_transfer() {
    let mint = br#"{"p":"brc-20","op":"mint","tick":"abcd","amt":"1"}"#;
    assert!(matches!(TokenCache::try_parse(ct(), mint, coin()), Ok(Brc4::Mint { .. })));

    let transfer = br#"{"p":"brc-20","op":"transfer","tick":"abcd","amt":"1"}"#;
    assert!(matches!(TokenCache::try_parse(ct(), transfer, coin()), Ok(Brc4::Transfer { .. })));
}
