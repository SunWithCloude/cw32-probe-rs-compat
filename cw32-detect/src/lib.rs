//! cw32-detect —— probe-rs 的 CW32(武汉芯源)芯片**自动识别层**。
//!
//! 背景: probe-rs 的 ARM 芯片自动识别依赖厂商 vendor 模块或 SWD multidrop,
//! CW32 全系列为 Cortex-M0+ + MINDP(DPv1), 在标准 ARM 调试组件层面无厂商
//! 唯一信息, 因此 `probe-rs info` 无法自动识别(见 probe-rs issue #4550)。
//!
//! 本 crate 利用 CW32 出厂区「数字签名」(ROM_Factory, 各系列 SDK 的
//! `cw32xx_digitalsign.h` 定义)实现识别: 型号串(ASCII) + 管脚数 + Flash/RAM
//! 容量 + UID 全部可经调试端口直接读取。
//!
//! 三种使用方式:
//! 1. **命令行** `cw32-detect`: 独立识别工具, 输出芯片名, 配合
//!    `probe-rs <cmd> --chip $(cw32-detect -q) ...` 使用;
//! 2. **库** [`detect::detect`]: 在你自己的 probe-rs 程序中调用;
//! 3. **插件** (feature `vendor`): [`vendor::register_vendor`] +
//!    [`vendor::register_targets_from_dir`] 把 CW32 识别注入本进程的
//!    probe-rs 自动识别链(基于 probe-rs 0.32 公开的 plugin API)。

pub mod detect;
pub mod table;

#[cfg(feature = "vendor")]
pub mod vendor;

pub use detect::{detect, Cw32Report, MatchMode};
