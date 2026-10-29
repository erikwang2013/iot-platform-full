#!/usr/bin/env python3
"""从中文模板生成英文 README 与 SVG 图表（文档仅保留中文 / English）。

用法: python3 scripts/gen_i18n_docs.py
输入: README.md, docs/*.zh.svg（中文源模板）
输出: docs/i18n/README.en.md, docs/{name}.en.svg
"""
import importlib
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DOCS = ROOT / "docs"
I18N = DOCS / "i18n"
SRC_DIR = Path(__file__).resolve().parent / "i18n"

LANGS = [("en", "English")]
NATIVE = {"zh": "中文", **dict(LANGS)}
SVGS = ["architecture", "flow", "features", "lifecycle", "security"]

TEXT_RE = re.compile(r"(<text[^>]*>)(.*?)(</text>)", re.S)
ATTR_RE = re.compile(r'\b(alt|title)="([^"]*)"')
MD_IMG_RE = re.compile(r"!\[([^\]]*)\]\(([^)]*)\)")
BAR_RE = re.compile(r"^\[中文\]\(README\.md\) \| \[English\]\([^)]*\).*$", re.M)
SVG_PATH_RE = re.compile(r"docs/([\w.-]+)\.zh\.svg")
DOCS_PATH_RE = re.compile(r'(?<=\]\()docs/|(?<=src=")docs/')

# 图片 alt/title 覆盖表：短键与 SVG <text> 标题共用 dict 键会冲突，故单独定义
ALT_OVERRIDE = {
    "en": {"架构图": "Architecture", "流程图": "Flows", "生命周期图": "Lifecycle", "安全架构": "Security", "微信支付": "WeChat Pay", "支付宝": "Alipay"},
}


def translate(s, table):
    return table.get(s.strip(), s)


def lang_bar(code):
    parts = []
    for c in ["zh", *[l[0] for l in LANGS]]:
        name = NATIVE[c]
        if c == code:
            target = "README.md" if c == "zh" else f"README.{c}.md"
        elif c == "zh":
            target = "README.md" if code == "zh" else "../../README.md"
        else:
            target = f"docs/i18n/README.{c}.md" if code == "zh" else f"README.{c}.md"
        parts.append(f"[{name}]({target})")
    return " | ".join(parts)


def gen_readme(code, table):
    src = (ROOT / "README.md").read_text(encoding="utf-8")
    text = BAR_RE.sub(lambda m: lang_bar(code), src)
    alt_table = {**table, **ALT_OVERRIDE.get(code, {})}
    text = ATTR_RE.sub(lambda m: f'{m.group(1)}="{translate(m.group(2), alt_table)}"', text)
    text = MD_IMG_RE.sub(
        lambda m: f"![{translate(m.group(1), alt_table)}]({m.group(2)})", text)
    # 整行翻译须在路径重写之前，否则 docs/ 被改写后字典键失配
    text = "\n".join(translate(line, table) for line in text.split("\n"))
    text = SVG_PATH_RE.sub(lambda m: f"../{m.group(1)}.{code}.svg", text)
    # 只改写链接目标，目录树和行内代码中的 docs/ 是仓库路径说明
    text = DOCS_PATH_RE.sub("../", text)
    (I18N / f"README.{code}.md").write_text(text, encoding="utf-8")


def gen_svg(name, code, table):
    src = (DOCS / f"{name}.zh.svg").read_text(encoding="utf-8")
    def repl(m):
        t = translate(m.group(2), table)
        return m.group(1) + t.replace("&", "&amp;") + m.group(3)
    text = TEXT_RE.sub(repl, src)
    (DOCS / f"{name}.{code}.svg").write_text(text, encoding="utf-8")


def main():
    sys.path.insert(0, str(SRC_DIR))
    tables = {code: importlib.import_module(f"zh_{code}").T for code, _ in LANGS}
    missing = [code for code, t in tables.items() if not t]
    if missing:
        sys.exit(f"empty translation dict: {missing}")
    I18N.mkdir(exist_ok=True)
    for code, name in LANGS:
        gen_readme(code, tables[code])
        for svg in SVGS:
            gen_svg(svg, code, tables[code])
        print(f"{name}: README.{code}.md + 5 SVGs")


if __name__ == "__main__":
    main()
