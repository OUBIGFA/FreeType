//! 设置程序（Go）保存时会把整份 `Config` 发回服务进程，缺一个字段那一项就回到默认值。
//! 这份默认设置夹具是两种语言共用的契约：Rust 端核对它与 `Config::default()` 一致，
//! Go 端（`windows/app/protocol_contract_test.go`）核对读入再写出不丢字段、不改取值。
use freetype_protocol::Config;

const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/config-default.json"
);

#[test]
fn default_config_matches_the_shared_fixture() {
    let actual = serde_json::to_value(Config::default()).unwrap();
    let text = serde_json::to_string_pretty(&actual).unwrap() + "\n";
    // 增删设置字段后设置 FREETYPE_UPDATE_FIXTURES=1 重跑一次即可更新夹具，再同步 Go 结构。
    if std::env::var_os("FREETYPE_UPDATE_FIXTURES").is_some() {
        std::fs::write(FIXTURE, &text).unwrap();
    }
    let expected: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(FIXTURE).unwrap()).unwrap();
    assert_eq!(
        actual, expected,
        "Config 字段或默认值已变化；用 FREETYPE_UPDATE_FIXTURES=1 更新夹具，并同步 windows/app/protocol.go"
    );
}
