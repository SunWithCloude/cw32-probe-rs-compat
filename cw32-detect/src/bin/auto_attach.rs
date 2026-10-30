//! 自动识别链端到端验证:注册 WHXY vendor 后**不指定芯片名**直接 attach,
//! 走 probe-rs 自身的 `auto_determine_target` 流水线(`probe-rs info` /
//! `TargetSelector::Auto` 的同一条路径)。
//!
//! 预期输出: `auto-detected: CW32L012x8`。
//! 若不注册 vendor, 同一块板会得到
//! "The connected chip could not automatically be determined."(issue #4550)。

use std::process::ExitCode;

use probe_rs::config::TargetSelector;
use probe_rs::probe::list::Lister;

fn main() -> ExitCode {
    // ① 把 WHXY vendor 前置插入 probe-rs 的自动识别链
    cw32_detect::vendor::register_vendor();

    let lister = Lister::new();
    let probes = lister.list_all();
    let Some(info) = probes.first() else {
        eprintln!("未发现调试探针");
        return ExitCode::from(2);
    };
    eprintln!(
        "使用探针: {} (VID:PID {:04x}:{:04x})",
        info.identifier, info.vendor_id, info.product_id
    );
    let probe = match info.open() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("打开探针失败: {e}");
            return ExitCode::from(2);
        }
    };

    // ② 不给芯片名 —— 全部交给 probe-rs 自动识别
    match probe.attach(TargetSelector::Auto, probe_rs::Permissions::default()) {
        Ok(session) => {
            println!("auto-detected: {}", session.target().name);
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("attach(Auto) 失败: {e}");
            ExitCode::from(1)
        }
    }
}
