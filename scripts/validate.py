#!/usr/bin/env python3
"""双金标准校验: 生成 YAML vs 硬件验证参照 (PR4549 F030 / 上游 CW32L0)"""
import sys
import base64
import difflib

sys.path.insert(0, '/home/z/my-project/share/agent-model/6ae4b6f87b0bd48ed7762f80/scripts')
from cw32_gen import parse_flm, yaml_series, parse_pdsc

import yaml

class Loader(yaml.SafeLoader):
    pass

def _ignore(loader, suffix, node):
    if isinstance(node, yaml.ScalarNode):
        return loader.construct_scalar(node)
    if isinstance(node, yaml.SequenceNode):
        return loader.construct_sequence(node)
    return loader.construct_mapping(node)

for tag in ('!Arm', '!v1', '!Nvm', '!Ram', '!Riscv', '!NvmRegion', '!RamRegion'):
    yaml.add_multi_constructor(tag, _ignore, Loader)

def norm(doc):
    """规范化为可比较结构"""
    out = {'name': doc['name'], 'variants': []}
    for v in doc.get('variants', []):
        mm = []
        for m in v['memory_map']:
            mm.append((type(m).__name__ if not isinstance(m, dict) else 'Nvm?',
                       m.get('name'), m['range']['start'], m['range']['end']))
        out['variants'].append({'name': v['name'], 'map': mm})
    algos = []
    for a in doc.get('flash_algorithms', []):
        blob = a['instructions']
        if isinstance(blob, str):
            raw = base64.b64decode(blob)
        else:
            raw = bytes(blob)
        algos.append({
            'name': a['name'],
            'blob_len': len(raw),
            'blob': raw,
            'pcs': {k: a.get(k) for k in ('pc_init', 'pc_uninit', 'pc_program_page',
                                           'pc_erase_sector', 'pc_erase_all', 'pc_verify')},
            'data_section_offset': a.get('data_section_offset'),
            'props': {k: v for k, v in a['flash_properties'].items()
                      if k != 'address_range'} | {'arange': (a['flash_properties']['address_range']['start'],
                                                             a['flash_properties']['address_range']['end'])},
        })
    out['flash_algorithms'] = algos
    return out

def compare(mine_path, ref_path, label, blob_only=False):
    print(f"\n===== {label} =====")
    mine = norm(yaml.load(open(mine_path), Loader=Loader))
    ref = norm(yaml.load(open(ref_path), Loader=Loader))
    ok = True
    if not blob_only:
        if mine['name'] != ref['name']:
            print(f"  name: 我={mine['name']} 参照={ref['name']}")
    # 变体名对比
    mn = [v['name'] for v in mine['variants']]
    rn = [v['name'] for v in ref['variants']]
    if mn != rn:
        print(f"  变体名差异(命名差异可接受): 我={mn} 参照={rn}")
    # 内存映射对比(按名字配对)
    rd = {v['name']: v for v in ref['variants']}
    for v in mine['variants']:
        rv = rd.get(v['name'])
        if rv is None:
            continue
        if v['map'] != rv['map']:
            print(f"  内存映射差异 {v['name']}: 我={v['map']} 参照={rv['map']}")
            ok = False
    # 算法对比(按算法名配对)
    rby = {a['name']: a for a in ref['flash_algorithms']}
    for ma in mine['flash_algorithms']:
        ra = rby.get(ma['name'])
        if ra is None:
            print(f"  [?] 参照中无同名算法 {ma['name']}, 尝试首个")
            ra = ref['flash_algorithms'][0]
        i = mine['flash_algorithms'].index(ma)
        if ma['blob'] != ra['blob']:
            print(f"  [X] 算法{i} blob 不一致! len {ma['blob_len']} vs {ra['blob_len']}")
            for j, (x, y) in enumerate(zip(ma['blob'], ra['blob'])):
                if x != y:
                    print(f"      首个差异 @{j:#x}: {x:#x} vs {y:#x}")
                    break
            ok = False
        else:
            print(f"  [✓] 算法 blob 逐字节一致 ({ma['blob_len']} 字节)")
        if ma['pcs'] != ra['pcs']:
            print(f"  [X] pc 字段差异: 我={ma['pcs']} 参照={ra['pcs']}")
            ok = False
        else:
            print(f"  [✓] pc 入口一致: {ma['pcs']}")
        if ma['data_section_offset'] != ra['data_section_offset']:
            print(f"  [X] data_section_offset: {ma['data_section_offset']:#x} vs {ra['data_section_offset']:#x}")
            ok = False
        else:
            print(f"  [✓] data_section_offset = {ma['data_section_offset']:#x}")
        if ma['props'] != ra['props']:
            print(f"  [X] flash_properties 差异: 我={ma['props']} 参照={ra['props']}")
            ok = False
        else:
            print(f"  [✓] flash_properties 一致: page={ma['props'].get('page_size')} "
                  f"timeouts={ma['props'].get('program_page_timeout')}/{ma['props'].get('erase_sector_timeout')}"
                  f" sectors={ma['props'].get('sectors')} range={ma['props'].get('arange')}")
    print(f"  结论: {'全部通过' if ok else '存在差异'}")
    return ok

T = '/home/z/my-project/share/agent-model/6ae4b6f87b0bd48ed7762f80/tmp'
D = '/home/z/my-project/share/agent-model/6ae4b6f87b0bd48ed7762f80/download/cw32-probe-rs-compat'

r1 = compare(f'{D}/targets/CW32F030_Series.yaml', f'{T}/CW32F030_Series_pr4549.yaml',
             'F030: 我的生成 vs PR#4549(硬件已验证)')

# L012: 手动走一遍生成流程(上游跳过), 与上游 CW32L0_Series.yaml 对比算法段
ver, devices = parse_pdsc(f'{T}/series/CW32L012/WHXY.CW32L012_DFP.1.0.2_pack/WHXY.CW32L012_DFP.pdsc')
algo = parse_flm(f'{T}/series/CW32L012/WHXY.CW32L012_DFP.1.0.2_pack/Flash/FlashCW32L012.FLM')
yml = yaml_series('CW32L012 Series', ver, devices, algo, 'flashcw32l012')
open(f'{T}/CW32L012_mine.yaml', 'w').write(yml)
r2 = compare(f'{T}/CW32L012_mine.yaml', f'{T}/CW32L0_Series_upstream.yaml',
             'L012: 我的生成 vs 上游 CW32L0_Series.yaml(#3682 已合并)', blob_only=True)

sys.exit(0 if (r1 and r2) else 1)
