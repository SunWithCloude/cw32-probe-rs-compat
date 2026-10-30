#!/usr/bin/env python3
"""CW32 全系列资产提取: zip → digitalsign 头 + DFP pack(pdsc/FLM/SVD)"""
import sys, os, zipfile, glob, re

def safe_name(zinfo):
    name = zinfo.filename
    try:
        name = zinfo.filename.encode('cp437').decode('gbk')
    except Exception:
        pass
    return name

def extract_matches(zf, outdir, name_filter):
    out = []
    for info in zf.infolist():
        if info.is_dir():
            continue
        name = safe_name(info)
        base = os.path.basename(name)
        if not base:
            continue
        if name_filter(name):
            tgt = os.path.join(outdir, base)
            if os.path.exists(tgt):
                continue
            with zf.open(info) as src, open(tgt, 'wb') as dst:
                dst.write(src.read())
            out.append(tgt)
    return out

def main(root):
    zips = sorted(glob.glob(os.path.join(root, 'cwlibs', '*.zip')))
    for z in zips:
        series = os.path.basename(z).split('_')[0]
        sdir = os.path.join(root, 'series', series)
        os.makedirs(sdir, exist_ok=True)
        zf = zipfile.ZipFile(z)
        # digitalsign 相关文件
        ds = extract_matches(zf, sdir, lambda n: 'digitalsign' in n.lower())
        # pack 文件
        packs = extract_matches(zf, sdir, lambda n: n.lower().endswith('.pack'))
        # SVD 兜底(EWARM 目录里的)
        svds = extract_matches(zf, sdir, lambda n: n.lower().endswith('.svd'))
        # 解包 pack
        packdirs = []
        for p in packs:
            pd = p.replace('.pack', '_pack')
            os.makedirs(pd, exist_ok=True)
            try:
                with zipfile.ZipFile(p) as pz:
                    pz.extractall(pd)
                packdirs.append(pd)
            except Exception as e:
                print(f"  ! 解包失败 {p}: {e}")
        print(f"[{series}] digitalsign={len(ds)} pack={len(packs)} svd={len(svds)} -> {sdir}")

if __name__ == '__main__':
    main(sys.argv[1])
