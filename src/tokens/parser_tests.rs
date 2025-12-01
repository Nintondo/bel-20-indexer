use super::parser::TokenCache;
use super::proto::Brc4;
use super::structs::{Brc4ParseErr, OriginalTokenTick};
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
