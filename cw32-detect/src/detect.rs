//! CW32 出厂区数字签名检测核心(独立工具路径)。
//!
//! 识别流程: 逐候选基址尝试, 每个基址使用一个全新的 attach 周期
//! (detach → attach_to_unspecified → try_into_arm_debug_interface),
//! 天然从未映射地址造成的 AP sticky error 中恢复。

use probe_rs::Error;
use probe_rs::architecture::arm::dp::DpAddress;
use probe_rs::architecture::arm::memory::ArmMemoryInterface;
use probe_rs::architecture::arm::sequences::DefaultArmSequence;
use probe_rs::architecture::arm::{ArmDebugInterface, ArmError};
use probe_rs::probe::Probe;

use crate::table::{CANDIDATE_BASES, SERIES, SeriesEntry};

/// 变体名解析方式
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchMode {
    /// 型号串与变体名完全一致
    Exact,
    /// x 归一化匹配 (CW32L012C8 → CW32L012x8, 上游 L0 系列命名)
    NormalizedX,
    /// 出厂区 Flash 容量与变体 IROM 一致
    FlashSize,
    /// 前缀兜底(容量未知或不匹配)
    PrefixFallback,
}

/// 识别结果
#[derive(Debug, Clone)]
pub struct Cw32Report {
    /// 出厂区读到的原始型号串, 如 "CW32F030C8"
    pub type_string: String,
    /// 命中的数字签名基址
    pub digitalsign_base: u32,
    /// 出厂区 Flash 容量(字节)
    pub flash_size: Option<u32>,
    /// 出厂区 RAM 容量(字节)
    pub ram_size: Option<u32>,
    /// 封装管脚数
    pub pin_count: Option<u16>,
    /// UID (连续式 10B; F002/F003 为 5 个字段拼接)
    pub uid: Option<Vec<u8>>,
    /// 解析出的 probe-rs target 变体名(未知系列时为 None)
    pub chip_name: Option<String>,
    pub match_mode: Option<MatchMode>,
    /// 是否为 probe-rs ≥v0.31.0 内置系列(CW32L010/L011/L012)
    pub builtin: bool,
    /// 本仓库 targets/ 下的 target 文件名(内置系列为 None)
    pub target_file: Option<String>,
    pub warnings: Vec<String>,
}

/// 识别入口。
///
/// 探针所有权会被临时移出并在结束时归还(为每个候选基址重建调试接口),
/// 因此返回 `(probe, result)`。未识别时 result 为 `Ok(None)`,
/// 连接彻底失败时为 `Err`。
pub fn detect(mut probe: Probe) -> (Probe, Result<Option<Cw32Report>, Error>) {
    let mut last_err: Option<Error> = None;
    let mut any_success = false;

    for &base in CANDIDATE_BASES {
        let _ = probe.detach();
        if let Err(e) = attach_once(&mut probe) {
            last_err = Some(e);
            continue;
        }
        match probe.try_into_arm_debug_interface(DefaultArmSequence::create()) {
            Ok(mut iface) => {
                let outcome = read_single_base(&mut *iface, base);
                probe = iface.close();
                any_success = true;
                match outcome {
                    Ok(Some(rep)) => return (probe, Ok(Some(rep))),
                    Ok(None) => {}
                    Err(e) => last_err = Some(e),
                }
            }
            Err((returned, e)) => {
                probe = returned;
                last_err = Some(e.into());
            }
        }
    }

    let _ = probe.detach();
    if any_success {
        // 至少有一条候选路径成功完成读取(只是没命中 CW32 签名) → 未发现, 而非错误。
        (probe, Ok(None))
    } else {
        // 不可达: 候选列表非空, 且每一轮失败必然记录了错误。
        (probe, Err(last_err.expect("no success path recorded without an error")))
    }
}

fn attach_once(probe: &mut Probe) -> Result<(), Error> {
    if probe.attach_to_unspecified().is_ok() {
        return Ok(());
    }
    // SWD 复位序列不响应时, 尝试硬件复位下连接。
    let _ = probe.detach();
    probe.attach_to_unspecified_under_reset()
}

/// 在一个已连接的 ARM 调试接口上读取单个候选基址。
fn read_single_base(
    iface: &mut dyn ArmDebugInterface,
    base: u32,
) -> Result<Option<Cw32Report>, Error> {
    if iface.select_debug_port(DpAddress::Default).is_err() {
        return Ok(None);
    }
    let aps = iface.access_ports(DpAddress::Default)?;
    for ap in &aps {
        let mut buf = [0u8; 24];
        let read: Result<(), ArmError> = match iface.memory_interface(ap) {
            Ok(mut mem) => mem.read_8(u64::from(base), &mut buf),
            Err(e) => Err(e),
        };
        if read.is_err() {
            // 该基址未映射/总线错误 —— 不是签名区, 继续下一个候选。
            continue;
        }
        // probe-rs 0.32 未公开 FullyQualifiedApAddress, 无法在函数签名里点名该类型:
        // 这里改为把内存句柄(而非 iface+AP)传给解析函数, AP 类型全程依赖推断。
        let Ok(mut mem) = iface.memory_interface(ap) else {
            continue;
        };
        if let Some(rep) = parse_report(&mut *mem, base, &buf)? {
            return Ok(Some(rep));
        }
    }
    Ok(None)
}

/// 校验 "CW32" 前缀并填充报告字段。
fn parse_report(
    mem: &mut dyn ArmMemoryInterface,
    base: u32,
    buf: &[u8; 24],
) -> Result<Option<Cw32Report>, Error> {
    let Some(type_string) = parse_type_string(buf) else {
        return Ok(None);
    };
    let Some(entry) = SERIES.iter().find(|e| type_string.starts_with(e.prefix)) else {
        // 检测到 CW32 芯片, 但系列不在识别表中(可能是新系列)。
        return Ok(Some(Cw32Report {
            warnings: vec![format!(
                "检测到 CW32 型号串 {type_string}, 但该系列不在识别表中; \
                 请从其官方固件库的 digitalsign.h 提取布局后补充 table。"
            )],
            type_string,
            digitalsign_base: base,
            flash_size: None,
            ram_size: None,
            pin_count: None,
            uid: None,
            chip_name: None,
            match_mode: None,
            builtin: false,
            target_file: None,
        }));
    };

    let (flash, ram, pins, uid) = fill_fields(mem, entry);

    let mut warnings = Vec::new();
    if flash.is_none() {
        warnings.push("出厂区容量字段读取失败(仅影响交叉校验)".to_string());
    }
    let (chip_name, mode) = resolve(entry, &type_string, flash, &mut warnings);

    Ok(Some(Cw32Report {
        type_string,
        digitalsign_base: base,
        flash_size: flash,
        ram_size: ram,
        pin_count: pins,
        uid,
        chip_name: Some(chip_name),
        match_mode: Some(mode),
        builtin: entry.builtin,
        target_file: entry.target_file.map(String::from),
        warnings,
    }))
}

/// 从 24 字节原始数据解析型号串: 必须以 ASCII "CW32" 开头, 取可打印段。
pub(crate) fn parse_type_string(buf: &[u8; 24]) -> Option<String> {
    if !buf.starts_with(b"CW32") {
        return None;
    }
    let end = buf
        .iter()
        .position(|&b| !(b as char).is_ascii_graphic())
        .unwrap_or(buf.len());
    let s = String::from_utf8_lossy(&buf[..end]).trim_end().to_string();
    (s.len() >= 6).then_some(s)
}

fn fill_fields(
    mem: &mut dyn ArmMemoryInterface,
    e: &SeriesEntry,
) -> (Option<u32>, Option<u32>, Option<u16>, Option<Vec<u8>>) {
    // 字段宽度按系列区分: B 组(L010-L012) pin=u8 / ram=u16, 其余 pin=u16 / ram=u32
    let flash = read_u32le(mem, e.flash_base);
    let ram = if e.ram_width <= 2 {
        read_u16le(mem, e.ram_base).map(u32::from)
    } else {
        read_u32le(mem, e.ram_base)
    };
    let pins = if e.pin_width <= 1 {
        read_u8le(mem, e.pin_base).map(u16::from)
    } else {
        read_u16le(mem, e.pin_base)
    };
    let uid = read_uid(mem, e);
    (flash, ram, pins, uid)
}

pub(crate) fn read_u8le(mem: &mut dyn ArmMemoryInterface, addr: u32) -> Option<u8> {
    let mut b = [0u8; 1];
    mem.read_8(u64::from(addr), &mut b).ok()?;
    Some(b[0])
}

pub(crate) fn read_u32le(mem: &mut dyn ArmMemoryInterface, addr: u32) -> Option<u32> {
    let mut b = [0u8; 4];
    mem.read_8(u64::from(addr), &mut b).ok()?;
    Some(u32::from_le_bytes(b))
}

pub(crate) fn read_u16le(mem: &mut dyn ArmMemoryInterface, addr: u32) -> Option<u16> {
    let mut b = [0u8; 2];
    mem.read_8(u64::from(addr), &mut b).ok()?;
    Some(u16::from_le_bytes(b))
}

fn read_uid(mem: &mut dyn ArmMemoryInterface, e: &SeriesEntry) -> Option<Vec<u8>> {
    // 连续式 UID (F020/F030, L010-L012, L031/L052/L083/R031/W031)
    if let (Some(base), len) = (e.uid_base, e.uid_len) {
        if len > 0 {
            let mut b = vec![0u8; len];
            if mem.read_8(u64::from(base), &mut b).is_ok() {
                return Some(b);
            }
            return None;
        }
    }
    // 分离式 UID (F002/F003): lid(4B) + wid(1B) + row(2B) + col(2B) + sid(1B)
    if let (Some(l), Some(w), Some(r), Some(c), Some(s)) =
        (e.uid_lid, e.uid_wid, e.uid_row, e.uid_col, e.uid_sid)
    {
        let mut out = Vec::with_capacity(10);
        for (addr, n) in [(l, 4usize), (w, 1), (r, 2), (c, 2), (s, 1)] {
            let mut b = vec![0u8; n];
            mem.read_8(u64::from(addr), &mut b).ok()?;
            out.extend_from_slice(&b);
        }
        return Some(out);
    }
    None
}

fn resolve(
    entry: &SeriesEntry,
    t: &str,
    flash: Option<u32>,
    warnings: &mut Vec<String>,
) -> (String, MatchMode) {
    // 1. 精确匹配
    if let Some(v) = entry.variants.iter().find(|v| v.name == t) {
        return (v.name.to_string(), MatchMode::Exact);
    }
    // 2. 封装后缀剥离: 厂商型号串常带封装码(如 CW32L012C8T6 / CW32F030C8T7),
    //    逐前缀回退, 每个前缀先做精确匹配再做 x 归一化(上游 L0 系列 "x8" 命名)。
    for cut in (6..t.len()).rev() {
        if !t.is_char_boundary(cut) {
            continue;
        }
        let p = &t[..cut];
        if let Some(v) = entry.variants.iter().find(|v| v.name == p) {
            return (v.name.to_string(), MatchMode::Exact);
        }
        if p.len() >= 2 && p.is_char_boundary(p.len() - 2) {
            let norm = format!("{}x{}", &p[..p.len() - 2], &p[p.len() - 1..]);
            if let Some(v) = entry.variants.iter().find(|v| v.name == norm) {
                return (v.name.to_string(), MatchMode::NormalizedX);
            }
        }
    }
    // 3. 出厂区 Flash 容量匹配
    if let Some(f) = flash {
        if let Some(v) = entry.variants.iter().find(|v| v.rom == f) {
            warnings.push(format!(
                "型号串 {t} 不在变体表中, 已按出厂区 Flash 容量 {f} 字节匹配到 {}",
                v.name
            ));
            return (v.name.to_string(), MatchMode::FlashSize);
        }
    }
    // 4. 前缀兜底
    match entry.variants.first() {
        Some(v) => {
            warnings.push(format!(
                "按系列前缀兜底选择 {}(容量校验未通过, 请人工确认)",
                v.name
            ));
            (v.name.to_string(), MatchMode::PrefixFallback)
        }
        None => unreachable!("系列条目至少包含一个变体"),
    }
}
