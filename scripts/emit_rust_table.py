#!/usr/bin/env python3
"""
从 detection_table.json 生成 Rust 代码:
  1. cw32-detect/src/table.rs   —— 识别指纹库(独立 crate 内使用)
  2. upstream/whxy.rs           —— 可直接放入 probe-rs 源码树的 vendor 模块
数据来源: CW32 各系列官方固件库 cw32xx_digitalsign.h (出厂区数字签名布局)。

字段宽度(经各系列 digitalsign.c 核实):
  A 组 F020/F030           : pin=u16, flash=u32, ram=u32
  B 组 L010/L011/L012      : pin=u8,  flash=u32, ram=u16
  C 组 F002/F003           : pin=u16, flash=u32, ram=u32
  D 组 L031/L052/L083/R031/W031 : pin=u16, flash=u32, ram=u32
"""
import json
import os

BASE = os.path.abspath(__file__ + '/../../download/cw32-probe-rs-compat')

# (系列前缀) -> (pin_width, ram_width)
WIDTHS = {
    'CW32L010': (1, 2), 'CW32L011': (1, 2), 'CW32L012': (1, 2),
}

# 候选基址探测顺序: A(F020/F030) → D(L031/L052/L083/R031/W031) → B(L010-L012) → C(F002/F003)
GROUP_ORDER = [0x12610, 0x100A24, 0x1007E0, 0x1007D0]


def addr(v):
    return None if v is None else (int(v, 16) if isinstance(v, str) else int(v))


def num(v):
    if v is None:
        return 0
    return int(v, 16) if isinstance(v, str) else int(v)


def rst(s):
    """Rust 字符串字面量(双引号 + 转义)"""
    return '"' + s.replace('\\', '\\\\').replace('"', '\\"') + '"'


def opt_u32(v):
    return 'None' if v is None else f'Some({v:#x})'


def load():
    with open(os.path.join(BASE, 'detection_table.json')) as f:
        return json.load(f)['series']


def series_lines(s):
    ds = s['digitalsign']
    pin_w, ram_w = WIDTHS.get(s['chip_type_prefix'], (2, 4))
    lines = []
    lines.append('SeriesEntry {')
    lines.append(f"    prefix: {rst(s['chip_type_prefix'])},")
    lines.append(f"    builtin: {'true' if s['builtin'] else 'false'},")
    tf = rst(s['target_file']) if s['target_file'] else 'None'
    lines.append(f"    target_file: {'Some(' + tf + ')' if s['target_file'] else 'None'},")
    lines.append('    variants: &[')
    for v in s['variants']:
        lines.append(f"        Variant {{ name: {rst(v['name'])}, rom: {v['rom']} }},")
    lines.append('    ],')
    lines.append(f"    type_base: {addr(ds['type']):#x},")
    lines.append(f"    type_len: {num(ds['tlen'])},")
    lines.append(f"    pin_base: {addr(ds['pin']):#x},")
    lines.append(f"    pin_width: {pin_w},")
    lines.append(f"    flash_base: {addr(ds['flash']):#x},")
    lines.append(f"    ram_base: {addr(ds['ram']):#x},")
    lines.append(f"    ram_width: {ram_w},")
    lines.append(f"    uid_base: {opt_u32(addr(ds.get('uid')))},")
    lines.append(f"    uid_len: {num(ds.get('ulen'))},")
    for k in ('uid_lid', 'uid_wid', 'uid_row', 'uid_col', 'uid_sid'):
        lines.append(f"    {k}: {opt_u32(addr(ds.get(k)))},")
    lines.append('},')
    return lines


HEADER_TABLE = """//! CW32 出厂区「数字签名」识别指纹库
//!
//! 由 scripts/emit_rust_table.py 从 detection_table.json 自动生成,
//! 数据来源为各系列官方固件库的 cw32xx_digitalsign.h:
//!   - F020/F030                 : 0x00012610 (22B 型号串)
//!   - L031/L052/L083/R031/W031  : 0x00100A24 (22B)
//!   - L010/L011/L012            : 0x001007E0 (16B)
//!   - F002/F003                 : 0x001007D0 (18B, 分离式 UID)
//! 字段宽度经各系列 digitalsign.c 核实: 仅 B 组(L010-L012)为
//! pin=u8 / ram=u16, 其余均为 pin=u16 / ram=u32; flash 全组 u32。

/// 候选型号串基址(按命中概率排序)。逐个尝试, 首个以 ASCII "CW32" 开头者即命中。
pub(crate) const CANDIDATE_BASES: &[u32] = &[
    {cands},
];

pub(crate) struct Variant {{
    /// probe-rs target 变体名(与 targets/*_Series.yaml 中一致)
    pub name: &'static str,
    /// Flash 容量(字节), 来自官方 pdsc IROM1
    pub rom: u32,
}}

pub(crate) struct SeriesEntry {{
    /// 系列前缀, 用于匹配出厂区型号串(如 "CW32F030")
    pub prefix: &'static str,
    /// 是否为 probe-rs >= v0.31.0 内置系列(CW32L010/L011/L012)
    pub builtin: bool,
    /// 本仓库 targets/ 下的 target 文件名(内置系列为 None)
    pub target_file: Option<&'static str>,
    pub variants: &'static [Variant],
    // ---- 数字签名布局 ----
    /// 型号字符串基址
    pub type_base: u32,
    /// 型号串字段长度(含填充)
    pub type_len: usize,
    /// 管脚数字段(宽度见 pin_width)
    pub pin_base: u32,
    /// 管脚数字段宽度(字节): B 组为 1, 其余为 2
    pub pin_width: u8,
    /// Flash 容量字段(u32)
    pub flash_base: u32,
    /// RAM 容量字段(宽度见 ram_width)
    pub ram_base: u32,
    /// RAM 容量字段宽度(字节): B 组为 2, 其余为 4
    pub ram_width: u8,
    /// UID(连续 10 字节; F002/F003 为分离式, 此字段为 None)
    pub uid_base: Option<u32>,
    pub uid_len: usize,
    // F002/F003 分离式 UID: lid(4B) wid(1B) row(2B) col(2B) sid(1B)
    pub uid_lid: Option<u32>,
    pub uid_wid: Option<u32>,
    pub uid_row: Option<u32>,
    pub uid_col: Option<u32>,
    pub uid_sid: Option<u32>,
}}

pub(crate) static SERIES: &[SeriesEntry] = &[
"""


def emit_table_rs(series):
    cands = ', '.join(f'{b:#x}' for b in GROUP_ORDER)
    out = HEADER_TABLE.format(cands=cands)
    for s in series:
        out += '\n'.join('    ' + ln if ln != '},' else '    },' for ln in series_lines(s)) + '\n'
    out += '];\n'
    path = os.path.join(BASE, 'cw32-detect', 'src', 'table.rs')
    os.makedirs(os.path.dirname(path), exist_ok=True)
    open(path, 'w').write(out)
    print(f'生成 {path} ({len(series)} 系列)')


HEADER_WHXY = """//! Wuhan Xinyuan Semiconductor (WHXY) CW32 vendor support.
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

use crate::{{
    Error, MemoryInterface,
    architecture::arm::{{
        ArmChipInfo, ArmDebugInterface,
        dp::DpAddress,
        memory::ArmMemoryInterface,
    }},
    config::{{DebugSequence, Registry}},
    vendor::Vendor,
}};

/// Wuhan Xinyuan
#[derive(Debug)]
pub struct Whxy;

impl std::fmt::Display for Whxy {{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {{
        write!(f, "Wuhan Xinyuan")
    }}
}}

impl Vendor for Whxy {{
    fn try_create_debug_sequence(&self, _chip: &Chip) -> Option<DebugSequence> {{
        None // CW32 为标准 Cortex-M0+, 无需自定义调试序列
    }}

    fn try_detect_arm_chip(
        &self,
        registry: &Registry,
        probe: &mut dyn ArmDebugInterface,
        _chip_info: ArmChipInfo,
    ) -> Result<Option<String>, Error> {{
        // 尽力而为: 任何失败都返回 None, 不阻塞其它厂商识别路径。
        let Ok(Some((type_string, flash))) = read_digitalsign(probe) else {{
            return Ok(None);
        }};
        for candidate in candidates(&type_string, flash) {{
            if registry.get_target_by_name(&candidate).is_ok() {{
                return Ok(Some(candidate));
            }}
        }}
        Ok(None)
    }}
}}

/// 生成候选 target 名(按置信度排序):
/// 精确名 → x 归一化(CW32L012C8 → CW32L012x8) → 前缀+Flash 容量。
fn candidates(type_string: &str, flash: Option<u32>) -> Vec<String> {{
    let mut out = vec![type_string.to_string()];
    let n = type_string.len();
    if n >= 2 && type_string.is_char_boundary(n - 2) {{
        out.push(format!("{{}}x{{}}", &type_string[..n - 2], &type_string[n - 1..]));
    }}
    if let Some(entry) = SERIES.iter().find(|e| type_string.starts_with(e.prefix)) {{
        if let Some(f) = flash {{
            for v in entry.variants {{
                if v.rom == f {{
                    out.push(v.name.to_string());
                }}
            }}
        }}
    }}
    out
}}

/// 扫描全部候选基址, 返回 (型号串, Flash 容量)。
fn read_digitalsign(probe: &mut dyn ArmDebugInterface) -> Result<Option<(String, Option<u32>)>, Error> {{
    for &base in CANDIDATE_BASES {{
        let aps = probe.access_ports(DpAddress::Default)?;
        let mut errored = false;
        for ap in &aps {{
            let mut buf = [0u8; 24];
            let read: Result<(), _> = match probe.memory_interface(ap) {{
                Ok(mut mem) => mem.read_8(u64::from(base), &mut buf),
                Err(e) => Err(e),
            }};
            match read {{
                Ok(()) => {{
                    if buf.starts_with(b"CW32") {{
                        let end = buf.iter().position(|&b| !(b as char).is_ascii_graphic()).unwrap_or(buf.len());
                        let type_string = String::from_utf8_lossy(&buf[..end]).trim_end().to_string();
                        if type_string.len() < 6 {{
                            continue;
                        }}
                        // 读 Flash 容量(交叉校验/变体选择)
                        let flash = probe
                            .memory_interface(ap)
                            .ok()
                            .and_then(|mut mem| read_u32le(&mut mem, base_flashing_addr(&type_string)));
                        return Ok(Some((type_string, flash)));
                    }}
                }}
                Err(_) => errored = true,
            }}
        }}
        if errored {{
            // 未映射地址可能使 AP 进入 sticky error, 重新初始化后继续。
            let _ = probe.reinitialize();
        }}
    }}
    Ok(None)
}}

fn base_flashing_addr(type_string: &str) -> u32 {{
    SERIES
        .iter()
        .find(|e| type_string.starts_with(e.prefix))
        .map(|e| e.flash_base)
        .unwrap_or(0)
}}

fn read_u32le(mem: &mut dyn ArmMemoryInterface, addr: u32) -> Option<u32> {{
    let mut b = [0u8; 4];
    mem.read_8(u64::from(addr), &mut b).ok()?;
    Some(u32::from_le_bytes(b))
}}

struct Variant {{
    name: &'static str,
    rom: u32,
}}

struct SeriesEntry {{
    prefix: &'static str,
    variants: &'static [Variant],
    type_base: u32,
    flash_base: u32,
}}

const CANDIDATE_BASES: &[u32] = &[
    {cands},
];

static SERIES: &[SeriesEntry] = &[
"""

FOOTER_WHXY = """];

// NOTE: SeriesEntry 中保留 type_base 便于对照排查; pin/ram/UID 字段本模块
// 未使用(仅识别 target 名), 完整布局见 cw32-probe-rs-compat/detection_table.json。
"""


def emit_whxy_rs(series):
    cands = ', '.join(f'{b:#x}' for b in GROUP_ORDER)
    out = HEADER_WHXY.format(cands=cands)
    for s in series:
        ds = s['digitalsign']
        out += '    SeriesEntry {\n'
        out += f"        prefix: {rst(s['chip_type_prefix'])},\n"
        out += '        variants: &[\n'
        for v in s['variants']:
            out += f"            Variant {{ name: {rst(v['name'])}, rom: {v['rom']} }},\n"
        out += '        ],\n'
        out += f"        type_base: {addr(ds['type']):#x},\n"
        out += f"        flash_base: {addr(ds['flash']):#x},\n"
        out += '    },\n'
    out += FOOTER_WHXY
    path = os.path.join(BASE, 'upstream', 'whxy.rs')
    os.makedirs(os.path.dirname(path), exist_ok=True)
    open(path, 'w').write(out)
    print(f'生成 {path} ({len(series)} 系列)')


if __name__ == '__main__':
    series = load()
    emit_table_rs(series)
    emit_whxy_rs(series)
