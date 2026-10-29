#!/usr/bin/env python3
"""Check key consistency across shared ARB locale files, optionally against HarmonyOS string.json.

Exit codes: 0 = key sets consistent; 1 = missing locale file or key mismatch.
Empty values in non-zh files are warnings only and do not fail the check.

Usage:
  python3 scripts/check_l10n.py
  python3 scripts/check_l10n.py --harmony [<resources_dir>...]
  python3 scripts/check_l10n.py --harmony --fix   # sync keys into string.json

--harmony with no DIR checks both HarmonyOS apps' entry resource dirs.
--fix adds ARB keys missing from each locale's string.json (locale value, then
English, then Chinese fallback) and drops names no longer present in ARB.
Re-run it after ARB changes; it is idempotent.
"""
import argparse
import json
import re
import sys
from pathlib import Path

LOCALES = ["zh", "en", "ko", "ru", "de", "fr", "es", "pt", "hi", "ar", "bn", "id", "ja"]

# HarmonyOS resource sub-dir per locale ("base" = zh)
HARMONY_DIRS = {
    "zh": "base",
    "en": "en_US",
    "ko": "ko_KR",
    "ru": "ru_RU",
    "de": "de_DE",
    "fr": "fr_FR",
    "es": "es_ES",
    "pt": "pt_BR",
    "hi": "hi_IN",
    "ar": "ar_SA",
    "bn": "bn_BD",
    "id": "id_ID",
    "ja": "ja_JP",
}

ROOT = Path(__file__).resolve().parent.parent
ARB_DIR = ROOT / "apps" / "shared" / "l10n"

# --harmony without explicit dirs: both HarmonyOS apps' entry resource dirs.
DEFAULT_HARMONY_DIRS = [
    ROOT / "apps" / "admin" / "harmonyos" / "entry" / "src" / "main" / "resources",
    ROOT / "apps" / "client" / "harmonyos" / "entry" / "src" / "main" / "resources",
]


def camel_to_snake(name: str) -> str:
    return re.sub(r"(?<!^)(?=[A-Z])", "_", name).lower()


def _load(path: Path) -> dict:
    with open(path, encoding="utf-8") as f:
        return json.load(f)


def arb_keys(path: Path) -> dict:
    """Return key->value of an arb file, skipping "@" metadata keys."""
    return {k: v for k, v in _load(path).items() if not k.startswith("@")}


def arb_locale(path: Path) -> str | None:
    """Return the @@locale metadata value (may be missing)."""
    return _load(path).get("@@locale")


def placeholders(value: str) -> set[str]:
    """ICU placeholder names in a value ({{ }} escapes stripped first)."""
    text = str(value).replace("{{", "").replace("}}", "")
    return set(re.findall(r"\{([A-Za-z_]\w*)", text))


def check_arbs(arb_dir: Path) -> tuple[list[str], list[str]]:
    problems: list[str] = []
    warnings: list[str] = []
    zh_path = arb_dir / "app_zh.arb"
    if not zh_path.exists():
        return [f"missing base file: {zh_path}"], []

    zh = arb_keys(zh_path)
    empty_zh = [k for k, v in zh.items() if not str(v).strip()]
    if empty_zh:
        problems.append(f"app_zh.arb: empty values: {empty_zh}")

    # 13 locale files must exist; no extra/missing files
    seen = set()
    for p in sorted(arb_dir.glob("app_*.arb")):
        loc = p.stem.removeprefix("app_")
        seen.add(loc)
        if loc not in LOCALES:
            problems.append(f"unexpected locale file: {p.name}")
    for loc in LOCALES:
        if loc not in seen:
            problems.append(f"missing locale file: app_{loc}.arb")

    for loc in LOCALES:
        p = arb_dir / f"app_{loc}.arb"
        if not p.exists():
            continue
        locale = arb_locale(p)
        if locale != loc:
            problems.append(f"{p.name}: @@locale is {locale!r}, expected {loc!r}")
        keys = arb_keys(p)
        missing = sorted(set(zh) - set(keys))
        extra = sorted(set(keys) - set(zh))
        if missing or extra:
            problems.append(f"{p.name}: missing keys {missing}, extra keys {extra}")
        if loc != "zh":
            empty = [k for k in zh if not str(keys.get(k, "")).strip()]
            if empty:
                warnings.append(f"{p.name}: empty values (warning): {empty}")
            for k in zh:
                got = placeholders(keys.get(k, ""))
                want = placeholders(zh[k])
                if got != want:
                    problems.append(
                        f"{p.name}[{k}]: placeholders {sorted(got)}, "
                        f"expected {sorted(want)}"
                    )
    return problems, warnings


def sync_harmony(harmony_dir: Path, arb_dir: Path) -> list[str]:
    """Make each locale string.json match the ARB key set (updates in place).

    Existing entries keep their order and translations; missing keys are appended
    in sorted order with the locale value (falling back to en, then zh). Names no
    longer in the ARB are removed so the key-parity check can pass.
    """
    values: dict[str, dict[str, str]] = {}
    for loc in LOCALES:
        p = arb_dir / f"app_{loc}.arb"
        values[loc] = {camel_to_snake(k): str(v) for k, v in arb_keys(p).items()} if p.exists() else {}
    expected = sorted(values["zh"])
    en = values["en"]
    zh = values["zh"]
    report: list[str] = []

    for loc in LOCALES:
        target = harmony_dir / HARMONY_DIRS[loc] / "element" / "string.json"
        data = _load(target) if target.exists() else {"string": []}
        kept = []
        kept_names = set()
        seen_names = set()
        for item in data.get("string", []):
            name = str(item.get("name", ""))
            if name in seen_names:
                continue
            seen_names.add(name)
            if name in values["zh"]:
                kept.append(item)
                kept_names.add(name)
        missing = [name for name in expected if name not in kept_names]
        removed = sorted(seen_names - kept_names)

        for name in missing:
            value = values[loc].get(name) or en.get(name) or zh.get(name, "")
            kept.append({"name": name, "value": value})

        if missing or removed:
            data["string"] = kept
            target.parent.mkdir(parents=True, exist_ok=True)
            with open(target, "w", encoding="utf-8") as f:
                json.dump(data, f, ensure_ascii=False, indent=2)
                f.write("\n")
            report.append(
                f"{HARMONY_DIRS[loc]}: +{len(missing)} missing, -{len(removed)} extra"
            )
    return report


def check_harmony(harmony_dir: Path, arb_dir: Path) -> list[str]:
    zh = arb_keys(arb_dir / "app_zh.arb")
    expected = sorted(camel_to_snake(k) for k in zh)
    problems = []
    for loc in LOCALES:
        string_json = harmony_dir / HARMONY_DIRS[loc] / "element" / "string.json"
        if not string_json.exists():
            problems.append(f"{string_json.relative_to(harmony_dir)}: missing")
            continue
        with open(string_json, encoding="utf-8") as f:
            data = json.load(f)
        items = data.get("string", [])
        names = sorted(str(it["name"]) for it in items if "name" in it)
        missing = sorted(set(expected) - set(names))
        extra = sorted(set(names) - set(expected))
        if missing or extra:
            problems.append(
                f"{HARMONY_DIRS[loc]}: missing names {missing}, extra names {extra}"
            )
    return problems


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--harmony", nargs="*", metavar="DIR",
                    help="HarmonyOS resources dir(s) to validate against the arb key set "
                         "(default: both apps' entry resource dirs)")
    ap.add_argument("--fix", action="store_true",
                    help="sync missing keys into HarmonyOS string.json before checking")
    ap.add_argument("--arb-dir", metavar="DIR",
                    help="arb directory to check (default: apps/shared/l10n)")
    args = ap.parse_args()

    arb_dir = Path(args.arb_dir).resolve() if args.arb_dir else ARB_DIR
    if args.harmony is None:
        harmony_dirs: list[Path] = []
    elif args.harmony:
        harmony_dirs = [Path(d) for d in args.harmony]
    else:
        harmony_dirs = list(DEFAULT_HARMONY_DIRS)
    if args.fix:
        if not harmony_dirs:
            ap.error("--fix requires --harmony")
        for d in harmony_dirs:
            for line in sync_harmony(d, arb_dir):
                print(f"SYNC [{d}] {line}")
    problems, warnings = check_arbs(arb_dir)
    for w in warnings:
        print(f"WARNING: {w}")
    for p in problems:
        print(f"ERROR: {p}")

    count = len(list(arb_dir.glob("app_*.arb")))
    status = "OK" if not problems else "MISMATCH"
    print(f"{count} arb files checked, key sets vs app_zh.arb: {status}")
    code = 1 if problems else 0

    if harmony_dirs:
        for d in harmony_dirs:
            hp = check_harmony(d, arb_dir)
            for p in hp:
                print(f"ERROR [{d}]: {p}")
            if hp:
                code = 1
    sys.exit(code)


if __name__ == "__main__":
    main()
