#!/usr/bin/env python3
"""
CW32 → probe-rs 兼容层生成器
  输入:  官方 DFP pack (pdsc + FLM)
  输出:  probe-rs target YAML + 芯片识别表(数字签名布局)
转换规则与 probe-rs 上游 target-gen 一致, 并以两份硬验证参照校准:
  - upstream CW32L0_Series.yaml (probe-rs #3682, DFP CW32L012 1.0.2)
  - PR #4549 CW32F030_Series.yaml (SunWithCloude, DFP 1.0.5, 硬件验证)
"""
import os
import re
import sys
import json
import struct
import base64
import zipfile
import xml.etree.ElementTree as ET

ROOT = os.path.abspath(__file__ + '/../../tmp')
OUT = os.path.abspath(__file__ + '/../../download/cw32-probe-rs-compat')

# ---------------------------------------------------------------- ELF32 解析
class Section:
    def __init__(self, name, addr, data, flags, typ):
        self.name, self.addr, self.data, self.flags, self.typ = name, addr, data, flags, typ

class Symbol:
    def __init__(self, name, value, size, secidx):
        self.name, self.value, self.size, self.secidx = name, value, size, secidx

def parse_elf32(path):
    d = open(path, 'rb').read()
    assert d[:4] == b'\x7fELF' and d[4] == 1 and d[5] == 1, f"{path}: 非 ELF32-LE"
    (e_shoff,) = struct.unpack_from('<I', d, 0x20)
    (e_shentsize, e_shnum, e_shstrndx) = struct.unpack_from('<HHH', d, 0x2E)
    shs = []
    for i in range(e_shnum):
        off = e_shoff + i * e_shentsize
        (nameoff, typ, flags, addr, offset, size, link, info, align, entsz) = \
            struct.unpack_from('<IIIIIIIIII', d, off)
        shs.append(dict(nameoff=nameoff, typ=typ, flags=flags, addr=addr,
                        offset=offset, size=size, link=link, entsz=entsz))
    shstr = shs[e_shstrndx]
    strtab = d[shstr['offset']:shstr['offset'] + shstr['size']]
    def cstr(t, o):
        e = t.find(b'\0', o)
        return t[o:e].decode('utf-8', 'replace')
    for s in shs:
        s['name'] = cstr(strtab, s['nameoff'])
    SHT_PROGBITS, SHT_SYMTAB, SHT_NOBITS = 1, 2, 8
    sections, symbols = [], []
    for idx, s in enumerate(shs):
        if s['typ'] == SHT_PROGBITS:
            sections.append(Section(s['name'], s['addr'], d[s['offset']:s['offset'] + s['size']], s['flags'], s['typ']))
        if s['typ'] == SHT_SYMTAB:
            strndx = shs[s['link']]
            st = d[strndx['offset']:strndx['offset'] + strndx['size']]
            n = s['size'] // 16
            for i in range(n):
                (noff, val, sz, info, other, sec) = struct.unpack_from('<IIIBBH', d, s['offset'] + i * 16)
                symbols.append(Symbol(cstr(st, noff), val, sz, sec))
    return sections, symbols

# ---------------------------------------------------------------- FLM → flash algorithm
def parse_flm(flm_path):
    sections, symbols = parse_elf32(flm_path)
    prgcode = next(s for s in sections if s.name == 'PrgCode')
    prgdata = next((s for s in sections if s.name == 'PrgData'), None)
    devdscr = next((s for s in sections if s.name == 'DevDscr'), None)
    blob = bytes(prgcode.data)
    data_off = None
    if prgdata is not None:
        blob += prgdata.data
        data_off = prgdata.addr
        assert prgdata.addr == prgcode.addr + len(prgcode.data), "PrgData 不紧跟 PrgCode"
    symmap = {s.name: s.value for s in symbols}
    # DevDscr → FlashDevice
    dev = {}
    if devdscr is not None:
        dd = devdscr.data
        dev['version'] = struct.unpack_from('<H', dd, 0x00)[0]
        dev['name'] = cstrz(dd, 0x02)
        dev['type'] = struct.unpack_from('<H', dd, 0x82)[0]
        dev['addr'] = struct.unpack_from('<I', dd, 0x84)[0]
        dev['size'] = struct.unpack_from('<I', dd, 0x88)[0]
        dev['page_size'] = struct.unpack_from('<I', dd, 0x8C)[0]
        dev['erased'] = struct.unpack_from('<I', dd, 0x94)[0]
        dev['prog_to'] = struct.unpack_from('<I', dd, 0x98)[0]
        dev['erase_to'] = struct.unpack_from('<I', dd, 0x9C)[0]
        sectors = []
        off = 0xA0
        while off + 8 <= len(dd):
            sz, ad = struct.unpack_from('<II', dd, off)
            if sz == 0xFFFFFFFF and ad == 0xFFFFFFFF:
                break
            sectors.append({'size': sz, 'address': ad})
            off += 8
        dev['sectors'] = sectors
    algo = {
        'blob': blob,
        'data_section_offset': data_off,
        'pc_init': symmap.get('Init'),
        'pc_uninit': symmap.get('UnInit'),
        'pc_erase_all': symmap.get('EraseChip'),
        'pc_erase_sector': symmap.get('EraseSector'),
        'pc_program_page': symmap.get('ProgramPage'),
        'pc_verify': symmap.get('Verify'),
        'dev': dev,
    }
    return algo

def cstrz(b, off):
    e = b.find(b'\0', off)
    return b[off:e].decode('ascii', 'replace').strip()

# ---------------------------------------------------------------- pdsc 解析
def _memattr(mem):
    return mem.get('name') or mem.get('id') or ''

def parse_pdsc(pdsc_path):
    r = ET.parse(pdsc_path).getroot()
    rel = r.find('releases/release')
    ver = rel.get('version') if rel is not None else '?'
    devices = []
    for fam in r.iter('family'):
        proc = fam.find('processor')
        dcore = proc.get('Dcore') if proc is not None else 'Cortex-M0+'
        for sf in fam.iter('subFamily'):
            subname = sf.get('DsubFamily') or ''
            for dev in sf.iter('device'):
                dname = dev.get('Dname')
                if not dname:
                    continue
                rom = ram = None
                algo = None
                for mem in dev.iter('memory'):
                    mid = _memattr(mem).upper()
                    if mid.startswith('IROM') or 'ROM' in mid:
                        rom = (int(mem.get('start'), 16), int(mem.get('size'), 16))
                    if mid.startswith('IRAM') or 'RAM' in mid:
                        ram = (int(mem.get('start'), 16), int(mem.get('size'), 16))
                for a in dev.iter('algorithm'):
                    algo = {'name': a.get('name'), 'start': int(a.get('start'), 16),
                            'size': int(a.get('size'), 16)}
                # 内存兜底: subFamily / family 层
                if rom is None or ram is None:
                    for scope in (dev, sf, fam):
                        for mem in scope.iter('memory'):
                            mid = _memattr(mem).upper()
                            if rom is None and ('ROM' in mid):
                                rom = (int(mem.get('start'), 16), int(mem.get('size'), 16))
                            if ram is None and ('RAM' in mid):
                                ram = (int(mem.get('start'), 16), int(mem.get('size'), 16))
                devices.append({'name': dname, 'subfamily': subname, 'core': dcore,
                                'rom': rom, 'ram': ram, 'algo': algo})
    return ver, devices

def find_pdscs(sdir):
    out = []
    for base, dirs, files in os.walk(sdir):
        for f in files:
            if f.endswith('.pdsc'):
                out.append(os.path.join(base, f))
    return out

def lcp(names):
    p = names[0]
    for n in names[1:]:
        while not n.startswith(p):
            p = p[:-1]
    return p

# ---------------------------------------------------------------- YAML 生成 (对齐 target-gen 输出格式)
CORE_TYPE = {'Cortex-M0+': 'armv6m', 'Cortex-M0': 'armv6m',
             'Cortex-M3': 'armv7m', 'Cortex-M4': 'armv7em', 'Cortex-M33': 'armv8m'}

def yaml_series(family_name, pack_ver, devices, algo, algo_name):
    L = []
    L.append(f"name: {family_name}")
    L.append("generated_from_pack: true")
    L.append(f"pack_file_release: {pack_ver}")
    L.append("variants:")
    for dev in devices:
        rom, ram = dev['rom'], dev['ram']
        L.append(f"- name: {dev['name']}")
        L.append("  cores:")
        L.append("  - name: main")
        L.append(f"    type: {CORE_TYPE.get(dev['core'], 'armv6m')}")
        L.append("    core_access_options: !Arm")
        L.append("      ap: !v1 0")
        L.append("  memory_map:")
        L.append("  - !Nvm")
        L.append("    name: IROM1")
        L.append("    range:")
        L.append(f"      start: {rom[0]:#x}")
        L.append(f"      end: {rom[0] + rom[1]:#x}")
        L.append("    cores:")
        L.append("    - main")
        L.append("    access:")
        L.append("      write: false")
        L.append("      boot: true")
        L.append("  - !Ram")
        L.append("    name: IRAM1")
        L.append("    range:")
        L.append(f"      start: {ram[0]:#x}")
        L.append(f"      end: {ram[0] + ram[1]:#x}")
        L.append("    cores:")
        L.append("    - main")
        L.append("  flash_algorithms:")
        L.append(f"  - {algo_name}")
    L.append("flash_algorithms:")
    L.append(f"- name: {algo_name}")
    L.append(f"  description: Flash algorithm for {family_name} (converted from vendor FLM)")
    L.append("  default: true")
    L.append(f"  instructions: {base64.b64encode(algo['blob']).decode()}")
    for k in ('pc_init', 'pc_uninit', 'pc_program_page', 'pc_erase_sector', 'pc_erase_all', 'pc_verify'):
        v = algo.get(k)
        if v is not None:
            L.append(f"  {k}: {v:#x}")
    L.append(f"  data_section_offset: {algo['data_section_offset']:#x}")
    L.append("  flash_properties:")
    dv = algo['dev']
    a0 = dv['addr']; a1 = dv['addr'] + dv['size']
    L.append("    address_range:")
    L.append(f"      start: {a0:#x}")
    L.append(f"      end: {a1:#x}")
    L.append(f"    page_size: {dv['page_size']:#x}")
    L.append(f"    erased_byte_value: {dv['erased']:#x}")
    L.append(f"    program_page_timeout: {dv['prog_to']}")
    L.append(f"    erase_sector_timeout: {dv['erase_to']}")
    L.append("    sectors:")
    for s in dv['sectors']:
        L.append(f"    - size: {s['size']:#x}")
        L.append(f"      address: {s['address']:#x}")
    return "\n".join(L) + "\n"

# ---------------------------------------------------------------- 数字签名布局
# 从各系列 SDK 的 cw32xx_digitalsign.h 提取
SIGN_LAYOUTS = {
    'CW32F020':  dict(group='A', type=0x00012610, tlen=22, pin=0x00012626, flash=0x00012628, ram=0x0001262C, uid=0x00012660, ulen=10),
    'CW32F030':  dict(group='A', type=0x00012610, tlen=22, pin=0x00012626, flash=0x00012628, ram=0x0001262C, uid=0x00012660, ulen=10),
    'CW32F002':  dict(group='C', type=0x001007D0, tlen=18, pin=0x001007E2, flash=0x001007E4, ram=0x001007E8,
                       uid=None, ulen=0, uid_lid=0x0010084C, uid_wid=0x00100850, uid_row=0x00100854,
                       uid_col=0x00100858, uid_sid=0x0010085C),
    'CW32F003':  dict(group='C', type=0x001007D0, tlen=18, pin=0x001007E2, flash=0x001007E4, ram=0x001007E8,
                       uid=None, ulen=0, uid_lid=0x0010084C, uid_wid=0x00100850, uid_row=0x00100854,
                       uid_col=0x00100858, uid_sid=0x0010085C),
    'CW32L010':  dict(group='B', type=0x001007E0, tlen=16, pin=0x001007D5, flash=0x001007D8, ram=0x001007D6, uid=0x001007B0, ulen=10),
    'CW32L011':  dict(group='B', type=0x001007E0, tlen=16, pin=0x001007D5, flash=0x001007D8, ram=0x001007D6, uid=0x001007B0, ulen=10),
    'CW32L012':  dict(group='B', type=0x001007E0, tlen=16, pin=0x001007D5, flash=0x001007D8, ram=0x001007D6, uid=0x001007B0, ulen=10),
    'CW32L031':  dict(group='D', type=0x00100A24, tlen=22, pin=0x00100A3A, flash=0x00100A3C, ram=0x00100A40, uid=0x00100A10, ulen=10),
    'CW32L052':  dict(group='D', type=0x00100A24, tlen=22, pin=0x00100A3A, flash=0x00100A3C, ram=0x00100A40, uid=0x00100A10, ulen=10),
    'CW32L083':  dict(group='D', type=0x00100A24, tlen=22, pin=0x00100A3A, flash=0x00100A3C, ram=0x00100A40, uid=0x00100A10, ulen=10),
    'CW32R031':  dict(group='D', type=0x00100A24, tlen=22, pin=0x00100A3A, flash=0x00100A3C, ram=0x00100A40, uid=0x00100A10, ulen=10),
    'CW32W031':  dict(group='D', type=0x00100A24, tlen=22, pin=0x00100A3A, flash=0x00100A3C, ram=0x00100A40, uid=0x00100A10, ulen=10),
}

# 已在上游内置的系列(v0.31.0+), 不生成 YAML, 检测时映射到内置名
BUILTIN = {
    'CW32L010': 'CW32L010x8', 'CW32L011': 'CW32L011x8', 'CW32L012': 'CW32L012x8',
}

def main():
    targets_dir = os.path.join(OUT, 'targets')
    os.makedirs(targets_dir, exist_ok=True)
    detection = []
    made = []
    for sdir in sorted(os.path.join(ROOT, 'series', d) for d in os.listdir(os.path.join(ROOT, 'series'))):
        series = os.path.basename(sdir)
        builtin = series in BUILTIN
        if builtin:
            # 上游内置: 只出检测表条目, 变体名与上游 CW32L0_Series.yaml 保持一致
            detection.append({
                'series': series,
                'chip_type_prefix': series,
                'builtin': True,
                'target_file': None,
                'variants': [{'name': BUILTIN[series], 'rom': 0x10000, 'ram': 0x2000}],
                'digitalsign': {k: (f"{v:#x}" if isinstance(v, int) and v is not None else v)
                                 for k, v in SIGN_LAYOUTS[series].items() if k != 'group'},
            })
            print(f"[BUILTIN] {series:9s} → 上游内置变体 {BUILTIN[series]} (probe-rs ≥ v0.31.0)")
            continue
        pdscs = find_pdscs(sdir)
        if not pdscs:
            print(f"[skip] {series}: 无 pdsc")
            continue
        pdsc = pdscs[0]
        pack_root = os.path.dirname(pdsc)
        ver, devices = parse_pdsc(pdsc)
        names = [d['name'] for d in devices]
        subfams = {d.get('subfamily') or '' for d in devices}
        subfams.discard('')
        if len(subfams) == 1:
            prefix = next(iter(subfams))
        else:
            prefix = lcp(names)
        family = f"{prefix} Series"
        flm_rel = devices[0]['algo']['name']
        flm_path = os.path.join(pack_root, flm_rel)
        algo = parse_flm(flm_path)
        algo_name = 'flash' + prefix.lower()
        yml = yaml_series(family, ver, devices, algo, algo_name)
        out = os.path.join(targets_dir, f"{prefix}_Series.yaml")
        open(out, 'w').write(yml)
        made.append(out)
        # 检测表条目
        layout = SIGN_LAYOUTS[series]
        detection.append({
            'series': series,
            'chip_type_prefix': prefix,
            'builtin': False,
            'target_file': f"{prefix}_Series.yaml",
            'variants': [{'name': d['name'], 'rom': d['rom'][1], 'ram': d['ram'][1]} for d in devices],
            'digitalsign': {k: (f"{v:#x}" if isinstance(v, int) and v is not None else v)
                             for k, v in layout.items() if k != 'group'},
        })
        dv = algo['dev']
        print(f"[OK ] {series:9s} {family:16s} pack v{ver:6s} "
              f"devices={len(devices)} page={dv['page_size']:#x} sectors={dv['sectors'][:2]} "
              f"blob={len(algo['blob'])}B")
    with open(os.path.join(OUT, 'detection_table.json'), 'w') as f:
        json.dump({'series': detection}, f, indent=2, ensure_ascii=False)
    print(f"\n生成 target YAML {len(made)} 个 → {targets_dir}")
    for m in made:
        print("  ", os.path.basename(m))

if __name__ == '__main__':
    main()
