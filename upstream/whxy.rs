//! Wuhan Xinyuan Semiconductor (WHXY) CW32 vendor support.
//!
//! 芯片识别: CW32 全系列在出厂区("digital signature" / ROM_Factory)存有
//! ASCII 型号串 + 管脚数 + Flash/RAM 容量 + UID, 可经调试端口直接读取。
//! 本模块在 ARM 自动识别阶段读取该区域, 把型号串映射为 registry 中的
//! target 名, 使 `probe-rs info` / 自动识别(不带 --chip)对 CW32 生效。
//!
//! 布局数据来源: 各系列官方固件库 cw32xx_digitalsign.h
//!   - F020/F030                 : 0x00012610 (22B 型号串)
//!   - L031/L052/L083/R031/W031  : 0x00100A24 (22B)
//!   - L010/L011/L012            : 0x001007E0 (16B)
//!   - F002/F003                 : 0x001007D0 (18B)
//!
//! 接入方式(probe-rs 源码树):
//!   1. 本文件放入 probe-rs/src/vendor/whxy.rs
//!   2. probe-rs/src/vendor/mod.rs: `pub mod whxy;` 并在 VENDORS 列表中
//!      加入 `&whxy::Whxy`
//!   3. targets/ 下放入对应的 CW32*_Series.yaml(见 cw32-probe-rs-compat)

use probe_rs_target::Chip;

use crate::{
    Error, MemoryInterface,
    architecture::arm::{
        ArmChipInfo, ArmDebugInterface,
        dp::DpAddress,
        memory::ArmMemoryInterface,
    },
    config::{DebugSequence, Registry},
    vendor::Vendor,
};

/// Wuhan Xinyuan
#[derive(Debug)]
pub struct Whxy;

impl std::fmt::Display for Whxy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Wuhan Xinyuan")
    }
}

impl Vendor for Whxy {
    fn try_create_debug_sequence(&self, _chip: &Chip) -> Option<DebugSequence> {
        None // CW32 为标准 Cortex-M0+, 无需自定义调试序列
    }

    fn try_detect_arm_chip(
        &self,
        registry: &Registry,
        probe: &mut dyn ArmDebugInterface,
        _chip_info: ArmChipInfo,
    ) -> Result<Option<String>, Error> {
        // 尽力而为: 任何失败都返回 None, 不阻塞其它厂商识别路径。
        let Ok(Some((type_string, flash))) = read_digitalsign(probe) else {
            return Ok(None);
        };
        for candidate in candidates(&type_string, flash) {
            if registry.get_target_by_name(&candidate).is_ok() {
                return Ok(Some(candidate));
            }
        }
        Ok(None)
    }
}

/// 生成候选 target 名(按置信度排序):
/// 精确名 → x 归一化(CW32L012C8 → CW32L012x8) → 前缀+Flash 容量。
fn candidates(type_string: &str, flash: Option<u32>) -> Vec<String> {
    let mut out = vec![type_string.to_string()];
    let n = type_string.len();
    if n >= 2 && type_string.is_char_boundary(n - 2) {
        out.push(format!("{}x{}", &type_string[..n - 2], &type_string[n - 1..]));
    }
    if let Some(entry) = SERIES.iter().find(|e| type_string.starts_with(e.prefix)) {
        if let Some(f) = flash {
            for v in entry.variants {
                if v.rom == f {
                    out.push(v.name.to_string());
                }
            }
        }
    }
    out
}

/// 扫描全部候选基址, 返回 (型号串, Flash 容量)。
fn read_digitalsign(probe: &mut dyn ArmDebugInterface) -> Result<Option<(String, Option<u32>)>, Error> {
    for &base in CANDIDATE_BASES {
        let aps = probe.access_ports(DpAddress::Default)?;
        let mut errored = false;
        for ap in &aps {
            let mut buf = [0u8; 24];
            let read: Result<(), _> = match probe.memory_interface(ap) {
                Ok(mut mem) => mem.read_8(u64::from(base), &mut buf),
                Err(e) => Err(e),
            };
            match read {
                Ok(()) => {
                    if buf.starts_with(b"CW32") {
                        let end = buf.iter().position(|&b| !(b as char).is_ascii_graphic()).unwrap_or(buf.len());
                        let type_string = String::from_utf8_lossy(&buf[..end]).trim_end().to_string();
                        if type_string.len() < 6 {
                            continue;
                        }
                        // 读 Flash 容量(交叉校验/变体选择)
                        let flash = probe
                            .memory_interface(ap)
                            .ok()
                            .and_then(|mut mem| read_u32le(&mut mem, base_flashing_addr(&type_string)));
                        return Ok(Some((type_string, flash)));
                    }
                }
                Err(_) => errored = true,
            }
        }
        if errored {
            // 未映射地址可能使 AP 进入 sticky error, 重新初始化后继续。
            let _ = probe.reinitialize();
        }
    }
    Ok(None)
}

fn base_flashing_addr(type_string: &str) -> u32 {
    SERIES
        .iter()
        .find(|e| type_string.starts_with(e.prefix))
        .map(|e| e.flash_base)
        .unwrap_or(0)
}

fn read_u32le(mem: &mut dyn ArmMemoryInterface, addr: u32) -> Option<u32> {
    let mut b = [0u8; 4];
    mem.read_8(u64::from(addr), &mut b).ok()?;
    Some(u32::from_le_bytes(b))
}

struct Variant {
    name: &'static str,
    rom: u32,
}

struct SeriesEntry {
    prefix: &'static str,
    variants: &'static [Variant],
    type_base: u32,
    flash_base: u32,
}

const CANDIDATE_BASES: &[u32] = &[
    0x12610, 0x100a24, 0x1007e0, 0x1007d0,
];

static SERIES: &[SeriesEntry] = &[
    SeriesEntry {
        prefix: "CW32F002",
        variants: &[
            Variant { name: "CW32F002F3", rom: 16384 },
        ],
        type_base: 0x1007d0,
        flash_base: 0x1007e4,
    },
    SeriesEntry {
        prefix: "CW32F003",
        variants: &[
            Variant { name: "CW32F003F4", rom: 20480 },
        ],
        type_base: 0x1007d0,
        flash_base: 0x1007e4,
    },
    SeriesEntry {
        prefix: "CW32F020",
        variants: &[
            Variant { name: "CW32F020C6", rom: 32768 },
            Variant { name: "CW32F020K6", rom: 32768 },
            Variant { name: "CW32F020F6", rom: 32768 },
        ],
        type_base: 0x12610,
        flash_base: 0x12628,
    },
    SeriesEntry {
        prefix: "CW32F030",
        variants: &[
            Variant { name: "CW32F030C8", rom: 65536 },
            Variant { name: "CW32F030K8", rom: 65536 },
            Variant { name: "CW32F030F8", rom: 65536 },
            Variant { name: "CW32F030F6", rom: 32768 },
        ],
        type_base: 0x12610,
        flash_base: 0x12628,
    },
    SeriesEntry {
        prefix: "CW32L010",
        variants: &[
            Variant { name: "CW32L010x8", rom: 65536 },
        ],
        type_base: 0x1007e0,
        flash_base: 0x1007d8,
    },
    SeriesEntry {
        prefix: "CW32L011",
        variants: &[
            Variant { name: "CW32L011x8", rom: 65536 },
        ],
        type_base: 0x1007e0,
        flash_base: 0x1007d8,
    },
    SeriesEntry {
        prefix: "CW32L012",
        variants: &[
            Variant { name: "CW32L012x8", rom: 65536 },
        ],
        type_base: 0x1007e0,
        flash_base: 0x1007d8,
    },
    SeriesEntry {
        prefix: "CW32L031",
        variants: &[
            Variant { name: "CW32L031C8", rom: 65536 },
            Variant { name: "CW32L031K8", rom: 65536 },
            Variant { name: "CW32L031F8", rom: 65536 },
        ],
        type_base: 0x100a24,
        flash_base: 0x100a3c,
    },
    SeriesEntry {
        prefix: "CW32L052",
        variants: &[
            Variant { name: "CW32L052R8", rom: 65536 },
            Variant { name: "CW32L052C8", rom: 65536 },
        ],
        type_base: 0x100a24,
        flash_base: 0x100a3c,
    },
    SeriesEntry {
        prefix: "CW32L083",
        variants: &[
            Variant { name: "CW32L083VC", rom: 262144 },
            Variant { name: "CW32L083MC", rom: 262144 },
            Variant { name: "CW32L083RC", rom: 262144 },
            Variant { name: "CW32L083RB", rom: 131072 },
        ],
        type_base: 0x100a24,
        flash_base: 0x100a3c,
    },
    SeriesEntry {
        prefix: "CW32R031",
        variants: &[
            Variant { name: "CW32R031C8", rom: 65536 },
        ],
        type_base: 0x100a24,
        flash_base: 0x100a3c,
    },
    SeriesEntry {
        prefix: "CW32W031",
        variants: &[
            Variant { name: "CW32W031R8", rom: 65536 },
        ],
        type_base: 0x100a24,
        flash_base: 0x100a3c,
    },
];

// NOTE: SeriesEntry 中保留 type_base 便于对照排查; pin/ram/UID 字段本模块
// 未使用(仅识别 target 名), 完整布局见 cw32-probe-rs-compat/detection_table.json。
