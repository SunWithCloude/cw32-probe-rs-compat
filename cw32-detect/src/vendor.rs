//! probe-rs 0.32 插件式 CW32 vendor —— 把识别层注入**本进程**的 probe-rs。
//!
//! 原理: probe-rs 0.32 的芯片自动识别链 (`vendor::auto_determine_target`)
//! 依次询问各厂商 vendor 的 `try_detect_arm_chip`。其 plugin API
//! (`probe_rs::plugin::register_plugin`) 允许外部 crate 把自定义 vendor
//! **前置**插入该链。本模块即利用这一点注册 WHXY(武汉芯源) 识别器。
//!
//! 注意: 该机制只对本进程内使用 probe-rs 库的程序生效(你的应用、自定义工具);
//! 官方 `probe-rs` CLI 二进制不会加载外部插件 —— CLI 场景请用 `cw32-detect`
//! 命令行工具, 或走上游化路径(见 upstream/whxy.rs)。

use probe_rs::plugin::{Plugin, register_plugin};
use probe_rs::vendor::Vendor;
use probe_rs::{
    Error,
    architecture::arm::dp::DpAddress,
    architecture::arm::{ArmChipInfo, ArmDebugInterface, ArmError},
    config::{DebugSequence, Registry},
};
use probe_rs_target::{Chip, ChipFamily};

use crate::detect::{parse_type_string, read_u32le};
use crate::table::{CANDIDATE_BASES, SERIES};

static WHXY: Whxy = Whxy;

/// Wuhan Xinyuan (CW32) vendor 识别器
pub struct Whxy;

impl core::fmt::Display for Whxy {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "Wuhan Xinyuan")
    }
}

impl Vendor for Whxy {
    fn try_create_debug_sequence(&self, _chip: &Chip) -> Option<DebugSequence> {
        // CW32 全系列为标准 Cortex-M0+, 使用默认序列。
        None
    }

    fn try_detect_arm_chip(
        &self,
        registry: &Registry,
        probe: &mut dyn ArmDebugInterface,
        _chip_info: ArmChipInfo,
    ) -> Result<Option<String>, Error> {
        // best-effort: 任何失败都返回 None, 不阻塞其它识别路径。
        let Some((type_string, flash)) = scan_digitalsign(probe).ok().flatten() else {
            return Ok(None);
        };
        for candidate in candidate_names(&type_string, flash) {
            if registry.get_target_by_name(&candidate).is_ok() {
                return Ok(Some(candidate));
            }
        }
        Ok(None)
    }
}

/// 依优先级枚举候选 target 名: 精确 → x 归一化 → 容量匹配 → 前缀兜底。
fn candidate_names(t: &str, flash: Option<u32>) -> Vec<String> {
    let mut out = vec![t.to_string()];
    if let Some(entry) = SERIES.iter().find(|e| t.starts_with(e.prefix)) {
        if t.len() >= 2 && t.is_char_boundary(t.len() - 2) {
            out.push(format!("{}x{}", &t[..t.len() - 2], &t[t.len() - 1..]));
        }
        for v in entry.variants {
            if Some(v.rom) == flash {
                out.push(v.name.to_string());
            }
        }
        for v in entry.variants {
            out.push(v.name.to_string());
        }
    }
    out
}

/// 在已连接的 ARM 调试接口上扫描全部候选基址(单次调用路径)。
/// 读取未映射地址可能使 AP 置位 sticky error, 故每轮失败后 reinitialize。
fn scan_digitalsign(
    probe: &mut dyn ArmDebugInterface,
) -> Result<Option<(String, Option<u32>)>, Error> {
    for &base in CANDIDATE_BASES {
        let aps = probe.access_ports(DpAddress::Default)?;
        let mut had_error = false;
        for ap in &aps {
            let mut buf = [0u8; 24];
            let read: Result<(), ArmError> = match probe.memory_interface(ap) {
                Ok(mut mem) => mem.read_8(u64::from(base), &mut buf),
                Err(e) => Err(e),
            };
            match read {
                Ok(()) => {
                    if let Some(type_string) = parse_type_string(&buf) {
                        let flash = probe.memory_interface(ap).ok().and_then(|mut mem| {
                            SERIES
                                .iter()
                                .find(|e| type_string.starts_with(e.prefix))
                                .and_then(|e| read_u32le(&mut *mem, e.flash_base))
                        });
                        return Ok(Some((type_string, flash)));
                    }
                }
                Err(_) => had_error = true,
            }
        }
        if had_error {
            let _ = probe.reinitialize();
        }
    }
    Ok(None)
}

/// 注册 CW32 vendor 识别器(前置插入, 先于内置 vendor 生效)。
///
/// 之后本进程内 `Session`/`probe.attach(TargetSelector::Auto, ..)`、
/// `probe-rs info` 等价的库调用即可自动识别 CW32。
pub fn register_vendor() {
    register_plugin(Plugin {
        vendors: &[&WHXY as &'static dyn Vendor],
        image_formats: &[],
        targets: &[],
        probe_drivers: &[],
    });
}

/// 从目录加载 CW32 target 定义(`CW32*_Series.yaml`)并注册。
/// 注册后本进程内使用这些芯片无需再传 `--chip-description-path`。
///
/// 返回成功注册的 family 数量。
pub fn register_targets_from_dir(dir: &std::path::Path) -> Result<usize, String> {
    let entries = dir
        .read_dir()
        .map_err(|e| format!("读取目录 {} 失败: {e}", dir.display()))?;
    let mut families = Vec::new();
    for ent in entries.flatten() {
        let p = ent.path();
        let name_ok = p
            .file_name()
            .and_then(|s| s.to_str())
            .map(|s| s.starts_with("CW32") && s.ends_with(".yaml"))
            .unwrap_or(false);
        if !name_ok {
            continue;
        }
        // probe-rs-target 0.32 没有 ChipFamily::from_yaml_file; 上游 Registry 的
        // 加载方式就是 yaml_serde::from_str(见 registry.rs add_target_family_from_yaml)。
        let text = std::fs::read_to_string(&p)
            .map_err(|e| format!("读取 {} 失败: {e}", p.display()))?;
        match yaml_serde::from_str::<ChipFamily>(&text) {
            Ok(f) => families.push(f),
            Err(e) => return Err(format!("解析 {} 失败: {e}", p.display())),
        }
    }
    let n = families.len();
    register_plugin(Plugin {
        vendors: &[],
        image_formats: &[],
        targets: &families,
        probe_drivers: &[],
    });
    Ok(n)
}
