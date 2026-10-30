//! targets/ 下全部家族 YAML 必须能按 probe-rs-target 0.32 的 ChipFamily
//! schema(`deny_unknown_fields`)解析 —— 与上游 Registry 同一反序列化路径。
//!
//! 官方 CLI 不走本 crate, 但 `register_targets_from_dir` / `parse_family_file`
//! 是库用户(方式 D)的入口, 解析失败必须在这里暴露而不是运行时才炸。

#![cfg(feature = "vendor")]

use cw32_detect::vendor::parse_family_file;

#[test]
fn all_family_yamls_parse() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../targets");
    let mut n = 0;
    for ent in dir.read_dir().expect("targets/ 目录存在") {
        let p = ent.expect("目录项可读").path();
        let name = p.file_name().expect("文件名").to_str().expect("UTF-8");
        if !(name.starts_with("CW32") && name.ends_with(".yaml")) {
            continue;
        }
        let fam = parse_family_file(&p).unwrap_or_else(|e| panic!("{e}"));
        assert!(fam.name.starts_with("CW32"), "family name: {}", fam.name);
        assert!(!fam.variants.is_empty(), "{}: 无变体", fam.name);
        // 家族名形如 "CW32F002 Series", 变体名共享去掉 " Series" 后的前缀
        let stem = fam.name.trim_end_matches(" Series");
        assert!(
            fam.variants.iter().all(|v| v.name.starts_with(stem)),
            "{}: 变体名与家族名前缀不符",
            fam.name
        );
        n += 1;
    }
    assert_eq!(n, 9, "应解析 9 个家族 YAML(F030 与 PR #4549 逐字节一致)");
}
