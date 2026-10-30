//! cw32-detect —— probe-rs 的 CW32 芯片自动识别命令行工具。
//!
//! 读取 CW32 出厂区「数字签名」, 输出可直接用于 `--chip` 的芯片名:
//!
//! ```sh
//! # 识别并烧录(本地 target 文件系列)
//! probe-rs download --chip $(cw32-detect -q) \
//!     --chip-description-path targets/CW32F030_Series.yaml firmware.elf
//!
//! # 内置系列(CW32L010/L011/L012, probe-rs >= v0.31.0)
//! probe-rs download --chip $(cw32-detect -q) firmware.elf
//! ```

use std::process::ExitCode;

use cw32_detect::{detect, Cw32Report, MatchMode};
use probe_rs::probe::list::Lister;
use probe_rs::probe::DebugProbeInfo;

struct Args {
    probe: Option<String>,
    json: bool,
    quiet: bool,
    targets_dir: Option<String>,
}

fn main() -> ExitCode {
    match parse_args() {
        Ok(Some(args)) => match run(&args) {
            Ok(code) => code,
            Err(msg) => {
                eprintln!("{msg}");
                ExitCode::from(2)
            }
        },
        Ok(None) => {
            print_usage();
            ExitCode::SUCCESS
        }
        Err(msg) => {
            eprintln!("参数错误: {msg}\n");
            print_usage();
            ExitCode::from(2)
        }
    }
}

fn run(args: &Args) -> Result<ExitCode, String> {
    let lister = Lister::new();
    let probes = lister.list_all();
    if probes.is_empty() {
        return Err(
            "未发现调试探针: 请连接 ST-Link / DAPLink / CMSIS-DAP 等, \
             并确认 USB 权限(Linux 下 udev 规则)"
                .to_string(),
        );
    }
    let info = select_probe(&probes, args.probe.as_deref())?;
    if !args.quiet && !args.json {
        eprintln!(
            "使用探针: {} (VID:PID {:04x}:{:04x})",
            info.identifier, info.vendor_id, info.product_id
        );
    }

    let probe = info
        .open()
        .map_err(|e| format!("打开探针失败: {e}"))?;
    let (mut probe, result) = detect(probe);
    let _ = probe.detach();

    match result {
        Ok(Some(rep)) => {
            output(&rep, args);
            if rep.chip_name.is_some() {
                Ok(ExitCode::SUCCESS)
            } else {
                Ok(ExitCode::from(1))
            }
        }
        Ok(None) => {
            eprintln!("未识别到 CW32 芯片 (已扫描全部出厂数字签名基址)。");
            eprintln!("提示: 确认目标已供电、SWD 接线正确(SWCLK/SWDIO/GND);");
            eprintln!("      本工具仅识别 CW32 系列, 其它芯片请直接使用 probe-rs。");
            Ok(ExitCode::from(1))
        }
        Err(e) => Err(format!("连接或读取失败: {e}")),
    }
}

fn parse_args() -> Result<Option<Args>, String> {
    let mut args = Args {
        probe: None,
        json: false,
        quiet: false,
        targets_dir: None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--help" | "-h" => return Ok(None),
            "--json" => args.json = true,
            "--quiet" | "-q" => args.quiet = true,
            "--probe" | "-p" => {
                let v = it
                    .next()
                    .ok_or_else(|| "--probe 需要一个值(VID:PID[:序列号])".to_string())?;
                args.probe = Some(v);
            }
            "--targets-dir" => {
                let v = it
                    .next()
                    .ok_or_else(|| "--targets-dir 需要一个目录路径".to_string())?;
                args.targets_dir = Some(v);
            }
            other => return Err(format!("未知参数 '{other}'")),
        }
    }
    Ok(Some(args))
}

fn select_probe<'a>(
    probes: &'a [DebugProbeInfo],
    spec: Option<&str>,
) -> Result<&'a DebugProbeInfo, String> {
    let Some(s) = spec else {
        return probes.first().ok_or_else(|| "无可用探针".to_string());
    };
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() < 2 {
        return Err(format!("--probe 需为 VID:PID[:序列号] 形式, 收到 '{s}'"));
    }
    let vid = u16::from_str_radix(parts[0].trim_start_matches("0x"), 16)
        .map_err(|e| format!("VID 解析失败: {e}"))?;
    let pid = u16::from_str_radix(parts[1].trim_start_matches("0x"), 16)
        .map_err(|e| format!("PID 解析失败: {e}"))?;
    let serial = parts.get(2).map(|s| s.to_string());
    for p in probes {
        if p.vendor_id == vid && p.product_id == pid {
            match &serial {
                None => return Ok(p),
                Some(s) => {
                    if p.serial_number.as_deref() == Some(s.as_str()) {
                        return Ok(p);
                    }
                }
            }
        }
    }
    Err(format!("未找到匹配的探针 '{s}'"))
}

fn output(rep: &Cw32Report, args: &Args) {
    if args.json {
        println!("{}", report_json(rep));
        return;
    }
    if args.quiet {
        match &rep.chip_name {
            Some(name) => println!("{name}"),
            None => {
                eprintln!(
                    "检测到 CW32 型号串 {}, 但无法映射到已知 target 名",
                    rep.type_string
                );
            }
        }
        return;
    }

    println!("CW32 芯片识别成功");
    println!(
        "  型号串:       {}  (数字签名 @ {:#x})",
        rep.type_string, rep.digitalsign_base
    );
    if let Some(f) = rep.flash_size {
        println!("  Flash:        {} 字节 ({} KB)", f, f / 1024);
    }
    if let Some(r) = rep.ram_size {
        println!("  RAM:          {} 字节 ({} KB)", r, r / 1024);
    }
    if let Some(p) = rep.pin_count {
        println!("  封装管脚:     {}", p);
    }
    if let Some(u) = &rep.uid {
        println!(
            "  UID:          {}",
            u.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(":")
        );
    }
    match (&rep.chip_name, rep.match_mode) {
        (Some(n), Some(m)) => println!("  probe-rs 目标: {n}  (匹配方式: {m:?})"),
        _ => println!("  probe-rs 目标: <未知系列>"),
    }
    if rep.builtin {
        println!("  来源:         probe-rs 内置 (v0.31.0+, 无需额外 target 文件)");
        if let Some(n) = &rep.chip_name {
            println!("  用法:         probe-rs <命令> --chip {n} ...");
        }
    } else if let (Some(n), Some(f)) = (&rep.chip_name, &rep.target_file) {
        let dir = targets_dir_hint(args);
        println!("  来源:         本仓库 targets/ (非内置, 需 --chip-description-path)");
        println!("  用法:         probe-rs <命令> --chip {n} --chip-description-path {dir}/{f} ...");
    }
    for w in &rep.warnings {
        println!("  警告: {w}");
    }
}

fn targets_dir_hint(args: &Args) -> String {
    if let Some(d) = &args.targets_dir {
        return d.clone();
    }
    for cand in ["./targets", "../targets"] {
        if std::path::Path::new(cand).is_dir() {
            return cand.to_string();
        }
    }
    "<targets 目录>".to_string()
}

fn report_json(rep: &Cw32Report) -> String {
    let uid = rep
        .uid
        .as_ref()
        .map(|u| u.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(":"));
    let mode = match rep.match_mode {
        Some(MatchMode::Exact) => "exact",
        Some(MatchMode::NormalizedX) => "normalized-x",
        Some(MatchMode::FlashSize) => "flash-size",
        Some(MatchMode::PrefixFallback) => "prefix-fallback",
        None => "unknown",
    };
    let pins = rep
        .pin_count
        .map(|p| p.to_string())
        .unwrap_or_else(|| "null".into());
    format!(
        "{{\"chip\": {}, \"type_string\": {}, \"builtin\": {}, \"target_file\": {}, \
         \"flash\": {}, \"ram\": {}, \"pins\": {}, \"uid\": {}, \"digitalsign_base\": {}, \
         \"match\": {}, \"warnings\": [{}]}}",
        jstr(rep.chip_name.as_deref().unwrap_or("")),
        jstr(&rep.type_string),
        rep.builtin,
        jstr(rep.target_file.as_deref().unwrap_or("")),
        opt_num(rep.flash_size),
        opt_num(rep.ram_size),
        pins,
        match &uid {
            Some(u) => jstr(u),
            None => "null".to_string(),
        },
        rep.digitalsign_base,
        jstr(mode),
        rep.warnings.iter().map(|w| jstr(w)).collect::<Vec<_>>().join(", ")
    )
}

fn opt_num(v: Option<u32>) -> String {
    v.map(|x| x.to_string()).unwrap_or_else(|| "null".into())
}

fn jstr(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn print_usage() {
    println!("cw32-detect —— probe-rs 的 CW32 芯片自动识别工具(基于出厂区数字签名)");
    println!();
    println!("用法: cw32-detect [选项]");
    println!();
    println!("选项:");
    println!("  --probe <VID:PID[:序列号]>  选择调试探针, 如 --probe 0483:3748");
    println!("  --json                      以 JSON 输出完整识别结果");
    println!("  --quiet, -q                 仅输出芯片名(用于 --chip $(cw32-detect -q))");
    println!("  --targets-dir <目录>        targets/ 目录位置(用于打印完整用法提示)");
    println!("  --help, -h                  显示帮助");
    println!();
    println!("示例:");
    println!("  cw32-detect");
    println!("  cw32-detect --probe 0483:3748 --json");
    println!("  probe-rs download --chip $(cw32-detect -q) \\");
    println!("      --chip-description-path targets/CW32F030_Series.yaml firmware.elf");
}
