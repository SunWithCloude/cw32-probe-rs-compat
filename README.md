# cw32-probe-rs-compat

**武汉芯源 CW32 全系列 × probe-rs 兼容层**：补齐 probe-rs 对 CW32 的两层缺失支持——

1. **target 层**（烧录/下载/调试）：9 个系列的 target 定义（从官方 DFP pack 提取 Flash 算法转换）；
2. **自动识别层**（`probe-rs info` / 不带 `--chip` 的自动识别）：基于 CW32 出厂区「数字签名」的识别器。

---

## 背景：probe-rs 对 CW32 的现状

| 层 | 状态 | 来源 |
|---|---|---|
| CW32L010 / L011 / L012 target | **已内置**（probe-rs ≥ v0.31.0, 2026-01-17） | PR #3682（WaitFme, 2025-12-14 合入） |
| CW32F030 target | PR #4549（SunWithCloude, open, 硬件已验证） | 本仓库 `targets/CW32F030_Series.yaml` 与其逐字节一致 |
| 其余 9 个系列 target | **缺失** → 本仓库补齐 | F002 / F003 / F020 / L031 / L052 / L083 / R031 / W031 |
| 自动识别层（全部系列） | **缺失**（issue #4550：`probe-rs info` 报 `The connected chip could not automatically be determined`） | 本仓库 `cw32-detect` + `upstream/whxy.rs` |

CW32 无法被 probe-rs 原生自动识别的原因：全系列为 Cortex-M0+ + **MINDP（DPv1，无 SWD multidrop）**，ROM 表里只有 ARM 通用组件，标准 ARM 调试拓扑层面**没有任何厂商唯一信息**。probe-rs 的识别靠各厂商 vendor 模块读厂商私有 ID（如 ST 读 DBGMCU_IDCODE），CW32 此前无人实现。

**钥匙**：CW32 在出厂区（ROM_Factory，官方 SDK `cw32xx_digitalsign.h`）存有一整套「数字签名」，经调试端口可直接读取：

```
型号串(ASCII)   管脚数   Flash 容量   RAM 容量   UID
```

芯片自己在内存里「报上名号」——这正是厂商识别模块需要的东西。

---

## 目录结构

```
cw32-probe-rs-compat/
├── targets/                    # ① target 层: probe-rs target YAML(9 个系列)
│   ├── CW32F002_Series.yaml
│   ├── CW32F003_Series.yaml
│   ├── CW32F020_Series.yaml
│   ├── CW32F030_Series.yaml    #   与 PR #4549 逐字节一致(硬件已验证)
│   ├── CW32L031_Series.yaml
│   ├── CW32L052_Series.yaml
│   ├── CW32L083_Series.yaml
│   ├── CW32R031_Series.yaml
│   └── CW32W031_Series.yaml
├── cw32-detect/                # ② 识别层: 自动识别工具(Rust crate: CLI + 库 + 插件)
│   ├── Cargo.toml
│   └── src/{main.rs, lib.rs, detect.rs, table.rs, vendor.rs}
├── upstream/                   # ③ 上游化: 可直接提交给 probe-rs 的 vendor 模块
│   └── whxy.rs
├── detection_table.json        # 识别指纹库(数字签名布局, 12 个系列)
├── scripts/                    # 全部生成脚本(可复现)
│   ├── extract_all.py          #   官方固件库 zip → pack/FLM/SVD 提取
│   ├── cw32_gen.py             #   pdsc+FLM → target YAML + detection_table.json
│   ├── emit_rust_table.py      #   detection_table.json → table.rs / whxy.rs
│   └── validate.py             #   双金标准校验(见下)
└── README.md
```

---

## 快速开始

### A. CW32L010 / L011 / L012（上游已内置）

```sh
probe-rs download --chip CW32L012x8 firmware.elf    # probe-rs ≥ v0.31.0
```

### B. 其它系列（本地 target 文件）

```sh
# 在本仓库根目录下
probe-rs download --chip CW32F030C8 \
    --chip-description-path targets/CW32F030_Series.yaml firmware.elf

# cargo-embed / cargo-flash 同样支持 --chip-description-path
# VSCode probe-rs 调试器: launch.json 的 chipDescriptionPath 字段
```

变体名（与 `probe-rs chip list` 风格一致，来自官方 pdsc 器件表）：

| 系列 | 变体 |
|---|---|
| CW32F030 | CW32F030C8 / K8 / F8（64K）、CW32F030F6（32K） |
| CW32F020 | CW32F020C6 / K6 / F6（32K） |
| CW32F003 | CW32F003F4（20K） |
| CW32F002 | CW32F002F3（16K） |
| CW32L031 | CW32L031C8 / K8 / F8（64K） |
| CW32L052 | CW32L052R8 / C8（64K） |
| CW32L083 | CW32L083VC / MC / RC（256K）、CW32L083RB（128K） |
| CW32R031 | CW32R031C8（64K） |
| CW32W031 | CW32W031R8（64K） |

### C. 自动识别工具 `cw32-detect`（本仓库核心交付）

```sh
cd cw32-detect
cargo build --release            # 需要 Rust 工具链; probe-rs = 0.32

# 识别(人类可读输出)
./target/release/cw32-detect
#   CW32 芯片识别成功
#     型号串:      CW32F030C8   (数字签名 @ 0x12610)
#     Flash:       65536 字节 (64 KB)
#     RAM:         8192 字节 (8 KB)
#     封装管脚:    48
#     UID:         12:34:...
#     probe-rs 目标: CW32F030C8   (匹配方式: Exact)
#     来源:        本仓库 targets/ (非内置, 需 --chip-description-path)
#     用法:        probe-rs <命令> --chip CW32F030C8 --chip-description-path targets/CW32F030_Series.yaml ...

# 一步烧录(组合使用)
probe-rs download --chip $(./target/release/cw32-detect -q) \
    --chip-description-path targets/CW32F030_Series.yaml firmware.elf

# 其它选项
cw32-detect --probe 0483:3748     # 指定探针(VID:PID[:序列号])
cw32-detect --json                # JSON 输出(便于脚本集成)
```

退出码：`0` 识别成功 / `1` 未发现 CW32 / `2` 探针错误。

识别覆盖 **全部 12 个系列**（含内置的 L010/L011/L012——识别后直接输出 `CW32L012x8` 这类内置变体名，无需 target 文件）。

### D. Rust 库用户：插件式注入（probe-rs 0.32 的公开 plugin API）

在自己使用 probe-rs 库的程序里，把 CW32 识别注入**本进程**的自动识别链：

```rust
fn main() {
    // ① 注册 WHXY vendor(前置插入 probe-rs 自动识别链)
    cw32_detect::vendor::register_vendor();
    // ② 可选: 注册全部 CW32 target(此后无需 --chip-description-path)
    cw32_detect::vendor::register_targets_from_dir("targets".as_ref()).unwrap();

    // 之后 Session::new(TargetSelector::Auto) 等自动识别路径对 CW32 生效
    let probe = probe_rs::Probe::open(...)?;
    let session = probe.attach("unspecified")?; // 或 attach_to_unspecified + Auto
    // ...
}
```

> 注：plugin 机制只对本进程生效；官方 `probe-rs` CLI 二进制不加载外部插件。
> CLI 场景请用 `cw32-detect`（方式 C）或走上游化（方式 E）。

### E. 上游化（根治方案）

把识别层做进 probe-rs 本体（正面回应 issue #4550 的问题 1）：

1. `upstream/whxy.rs` → `probe-rs/src/vendor/whxy.rs`，并在 `vendor/mod.rs` 的 `VENDORS` 列表加 `&whxy::Whxy`（模式与 WCH/ST 等 16 个厂商模块一致）；
2. `targets/CW32*_Series.yaml` → `probe-rs/probe-rs/targets/`（F030 部分即 PR #4549）；
3. 二者合入后：`probe-rs info`、不带 `--chip` 的自动识别、`chip list` 对 CW32 全系生效。

---

## 工作原理：出厂区数字签名

CW32 在系统存储区有出厂信息区（SDK 称 digital signature），四个地址布局组：

| 组 | 系列 | type_base | 宽度说明 |
|---|---|---|---|
| A | F020 / F030 | `0x0001_2610`（22B 串） | pin=u16 @+0x16, flash=u32 @+0x18, ram=u32 @+0x1C, uid@0x12660 |
| D | L031 / L052 / L083 / R031 / W031 | `0x0010_0A24`（22B） | pin=u16 @0x100A3A, flash=u32 @0x100A3C, ram=u32 @0x100A40, uid@0x100A10 |
| B | L010 / L011 / L012 | `0x0010_07E0`（16B） | **pin=u8** @0x1007D5, flash=u32 @0x1007D8, **ram=u16** @0x1007D6, uid@0x1007B0 |
| C | F002 / F003 | `0x0010_07D0`（18B） | pin=u16 @0x1007E2, flash=u32 @0x1007E4, ram=u32 @0x1007E8, UID 分离式(lid/wid/row/col/sid @0x10084C-0x10085C) |

识别算法（`cw32-detect`）：

1. 逐候选基址（A→D→B→C），每个基址用**全新 attach 周期**（`detach → attach_to_unspecified → try_into_arm_debug_interface`，均为 probe-rs 0.32 公开 API）——未映射地址造成的 AP sticky error 天然被下一周期清除；
2. 读 24 字节，判断是否以 ASCII `"CW32"` 开头（型号串是唯一会在这些位置出现的 ASCII 特征）；
3. 命中后按该系列布局读出 Flash/RAM 容量、管脚数、UID；
4. **变体名解析**（按置信度）：型号串精确匹配 → x 归一化（`CW32L012C8` → `CW32L012x8`，对齐上游 L0 命名）→ 出厂区 Flash 容量匹配 → 前缀兜底（附警告）。

target 层转换规则（与 probe-rs 官方 `target-gen` 一致，以两份硬验证参照校准）：

- `instructions = PrgCode + PrgData`（ELF 段拼接，**不含** DevDscr）；
- `pc_* = 符号地址（Init/UnInit/EraseChip/EraseSector/ProgramPage/Verify）`；
- `data_section_offset = PrgData 段起始地址`；
- `flash_properties` 来自 DevDscr 的 FlashDevice 结构（页大小/空值/超时/sector 表）。

---

## 验证情况

**双金标准校验（`scripts/validate.py`）**：

| 生成物 | 参照 | 结果 |
|---|---|---|
| `targets/CW32F030_Series.yaml` | PR #4549（SunWithCloude，CW32F030C8T7 + ST-Link V2 硬件验证） | 算法 blob **逐字节一致**（632 B）；pc 入口、data_section_offset(0x254)、flash_properties 全同 |
| `targets/CW32L031…` 同源流程产物 `flashcw32l012` | 上游已合并的 `CW32L0_Series.yaml`（#3682） | 算法 blob **逐字节一致**（412 B）；全字段同 |

**板上验证（2026-10-31，CW32L012C8T6 StartKit + ST-Link V2，probe-rs 0.32.0）**：

```
$ cw32-detect
CW32 芯片识别成功
  型号串:       CW32L012C8T6  (数字签名 @ 0x1007e0)
  Flash:        65536 字节 (64 KB)
  RAM:          8192 字节 (8 KB)
  封装管脚:     48
  UID:          73:00:AF:00:08:72:56:11:0C:87
  probe-rs 目标: CW32L012x8  (匹配方式: NormalizedX)
  来源:         probe-rs 内置 (v0.31.0+, 无需额外 target 文件)
  用法:         probe-rs <命令> --chip CW32L012x8 ...
```

实测发现厂商型号串带封装后缀（`…C8T6`），因此变体解析会逐前缀剥离封装码后再做精确 / x 归一化匹配（其它封装如 `CW32F030C8T7` 同样受益）。`cargo test` 含 9 家族 YAML 全量解析测试（走上游同款 `deny_unknown_fields` schema）。

**自动识别链验证（同日）**：`src/bin/auto_attach.rs` 通过 probe-rs 0.32 plugin API 注册 WHXY vendor（`register_plugin` 前置插入与内置识别同一条 `VENDORS` 链），以 `TargetSelector::Auto` attach——即 `probe-rs info` 背后的 `auto_determine_target` 路径——**不指定芯片名**，同一块此前报 `could not automatically be determined` 的板输出：

```
$ auto_attach   # register_vendor(); Probe::attach(TargetSelector::Auto, ..)
auto-detected: CW32L012x8
```

hook 内部实测：候选基址读取失败后 `reinitialize()` 即可清 MINDP sticky error，扫描得以继续。

识别所用的地址与字段宽度均逐系列核对过官方 `cw32xx_digitalsign.h/.c`；如某系列实测读数与本表有出入，请反馈实际读数，`detection_table.json` + `scripts/emit_rust_table.py` 可即时重生成。

**构建验证（2026-10-31）**：`cw32-detect` 对 crates.io `probe-rs = 0.32.0`（非 git 版）release 构建零错误零警告。为适配 0.32 公开 API 做了如下修正：

- `FullyQualifiedApAddress` 在 0.32 中为 crate 私有类型（`pub(crate) mod traits`，无公开导出路径）——识别流程改为把 `&mut dyn ArmMemoryInterface` 传入解析函数，AP 类型全程依赖推断；
- `ChipFamily` 无 `from_yaml_file` 关联函数——`register_targets_from_dir` 改用 `yaml_serde::from_str::<ChipFamily>` 加载（与上游 `Registry::add_target_family_from_yaml` 同一反序列化路径）。

---

## 上游与相关链接

- issue #4550 `probe-rs info cannot auto-detect the CW32L012 although the target is built in`（识别层空白；本仓库 `upstream/whxy.rs` 即其问题 1 的答案）
- PR #4549 `Add CW32F030 target support`（target 层，与本仓库 F030 文件一致）
- PR #3682 `Add support for cw32l0`（L0 系列内置，v0.31.0）
- probe-rs 0.32 的 vendor/plugin 机制（`register_plugin` 公开 API，`VENDORS` 前置插入）

## 已知限制

- `cw32-detect` 仅支持 **ARM/SWD** 路径的 CW32（全系列 Cortex-M0+，含 R031/W031）；RISC-V 芯片不适用。
- 依赖 probe-rs **0.32.x** API（`try_into_arm_debug_interface` 等）；0.33+ 如有 API 变更需小幅适配。
- W031 的 Flash 算法参数来自厂商 DevDscr：page=128B（其余系列 512B）、超时 10/1000ms——为官方真实值。
- F003 flash 为 20KB（0x5000，厂商 pdsc 原值，非 2 的幂）。
- 若芯片开启了读保护，数字签名区可能不可读——工具会报告未识别。

## 复现流程

```sh
# 1. 下载官方固件库(见 scripts/extract_all.py 内的 URL 列表, 来自 whxy.com 固件库页)
# 2. 解包提取
python3 scripts/extract_all.py <含各系列 zip 的目录的父目录>
# 3. 生成 target YAML + 识别表
python3 scripts/cw32_gen.py
# 4. 生成 Rust 识别表与上游 vendor 模块
python3 scripts/emit_rust_table.py
# 5. 校验(需参照文件: PR#4549 与上游 CW32L0_Series.yaml)
python3 scripts/validate.py
```
