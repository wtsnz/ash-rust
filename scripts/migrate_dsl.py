#!/usr/bin/env python3
"""Migrate resource! / quote! DSL to the frozen grammar."""

from __future__ import annotations

import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

SKIP_DIRS = {
    "target",
    ".git",
    "node_modules",
}


def is_ident_start(ch: str) -> bool:
    return ch.isalpha() or ch == "_"


def skip_ws_and_comments(src: str, i: int) -> int:
    n = len(src)
    while i < n:
        if src[i] in " \t\r\n":
            i += 1
            continue
        if src.startswith("//", i):
            i = src.find("\n", i)
            if i < 0:
                return n
            continue
        if src.startswith("/*", i):
            j = src.find("*/", i + 2)
            i = n if j < 0 else j + 2
            continue
        break
    return i


def parse_string(src: str, i: int) -> int:
    quote = src[i]
    i += 1
    n = len(src)
    while i < n:
        if src[i] == "\\":
            i += 2
            continue
        if src[i] == quote:
            return i + 1
        i += 1
    return n


def matching_delimiter(src: str, i: int) -> int:
    open_ch = src[i]
    close = {"{": "}", "(": ")", "[": "]"}[open_ch]
    i += 1
    n = len(src)
    depth = 1
    while i < n:
        ch = src[i]
        if ch in "\"'":
            i = parse_string(src, i)
            continue
        if src.startswith("//", i):
            j = src.find("\n", i)
            i = n if j < 0 else j
            continue
        if src.startswith("/*", i):
            j = src.find("*/", i + 2)
            i = n if j < 0 else j + 2
            continue
        if ch == open_ch:
            depth += 1
        elif ch == close:
            depth -= 1
            if depth == 0:
                return i
        i += 1
    return n - 1


def commas_to_semicolons_in_block(body: str) -> str:
    out = []
    i = 0
    n = len(body)
    bracket = 0
    paren = 0
    while i < n:
        ch = body[i]
        if ch in "\"'":
            j = parse_string(body, i)
            out.append(body[i:j])
            i = j
            continue
        if body.startswith("//", i):
            j = body.find("\n", i)
            j = n if j < 0 else j
            out.append(body[i:j])
            i = j
            continue
        if body.startswith("/*", i):
            j = body.find("*/", i + 2)
            j = n if j < 0 else j + 2
            out.append(body[i:j])
            i = j
            continue
        if ch == "[":
            bracket += 1
            out.append(ch)
        elif ch == "]":
            bracket = max(0, bracket - 1)
            out.append(ch)
        elif ch == "(":
            paren += 1
            out.append(ch)
        elif ch == ")":
            paren = max(0, paren - 1)
            out.append(ch)
        elif ch == "," and bracket == 0 and paren == 0:
            out.append(";")
        else:
            out.append(ch)
        i += 1
    return "".join(out)


SECTION_BLOCKS = (
    "attributes",
    "relationships",
    "calculations",
    "aggregates",
    "identities",
    "actor",
)


def rewrite_section_blocks(src: str) -> str:
    pattern = re.compile(r"\b(" + "|".join(SECTION_BLOCKS) + r")\s*\{")

    def repl(m: re.Match[str]) -> str:
        start = m.end() - 1
        end = matching_delimiter(src, start)
        body = src[start + 1 : end]
        return m.group(1) + " {" + commas_to_semicolons_in_block(body) + "}"

    # Apply repeatedly from left; use a manual scan to avoid overlapping issues.
    out = []
    i = 0
    n = len(src)
    while i < n:
        m = pattern.search(src, i)
        if not m:
            out.append(src[i:])
            break
        out.append(src[i : m.start()])
        start_brace = m.end() - 1
        end = matching_delimiter(src, start_brace)
        body = src[start_brace + 1 : end]
        out.append(m.group(1) + " {" + commas_to_semicolons_in_block(body) + "}")
        i = end + 1
    return "".join(out)


def transform_resource_macro(inner: str) -> str:
    s = inner
    # docs / attrs stay
    prefix = []
    rest = s
    while True:
        rest_lstrip = rest.lstrip()
        lead = len(rest) - len(rest_lstrip)
        prefix.append(rest[:lead])
        rest = rest_lstrip
        if rest.startswith("///") or rest.startswith("//!"):
            nl = rest.find("\n")
            if nl < 0:
                prefix.append(rest)
                rest = ""
                break
            prefix.append(rest[: nl + 1])
            rest = rest[nl + 1 :]
            continue
        if rest.startswith("#["):
            # find matching ]
            i = rest.find("[")
            end = matching_delimiter(rest, i)
            prefix.append(rest[: end + 1])
            rest = rest[end + 1 :]
            continue
        break

    pre = "".join(prefix)
    body = rest

    embedded = False
    m_emb = re.match(r"embedded\s*;\s*", body)
    if m_emb:
        embedded = True
        body = body[m_emb.end() :]
    elif re.match(r"embedded\s+[A-Z]", body):
        # already `embedded Name {`
        pass

    m_old = re.match(r"(?:resource|name)\s+([A-Za-z_][A-Za-z0-9_]*)\s*;?", body)
    if m_old:
        name = m_old.group(1)
        inner_body = body[m_old.end() :].strip()
        # strip a trailing leftover
        if embedded:
            return pre + f"embedded {name} {{\n        {inner_body}\n    }}"
        return pre + f"{name} {{\n        {inner_body}\n    }}"

    m_emb_name = re.match(r"embedded\s+([A-Za-z_][A-Za-z0-9_]*)\s*\{", body)
    if m_emb_name:
        return pre + body

    m_new = re.match(r"([A-Za-z_][A-Za-z0-9_]*)\s*\{", body)
    if m_new:
        return pre + body

    return inner


def transform_resource_macros_in_file(src: str) -> str:
    out = []
    i = 0
    n = len(src)
    needle = "resource!"
    while i < n:
        j = src.find(needle, i)
        if j < 0:
            out.append(src[i:])
            break
        out.append(src[i:j])
        k = skip_ws_and_comments(src, j + len(needle))
        if k < n and src[k] == "{":
            end = matching_delimiter(src, k)
            inner = src[k + 1 : end]
            new_inner = transform_resource_macro(inner=inner)
            # also rewrite section punctuation inside
            new_inner = rewrite_section_blocks(new_inner)
            out.append("resource! {")
            out.append(new_inner)
            out.append("}")
            i = end + 1
        else:
            out.append(src[j : j + len(needle)])
            i = j + len(needle)
    return "".join(out)


def transform_quote_headers(src: str) -> str:
    """Turn `quote! { resource Name; ... }` into `quote! { Name { ... } }`."""
    pattern = re.compile(r"quote!\s*\{")
    out = []
    i = 0
    n = len(src)
    while i < n:
        m = pattern.search(src, i)
        if not m:
            out.append(src[i:])
            break
        out.append(src[i : m.end() - 1])
        start = m.end() - 1
        end = matching_delimiter(src, start)
        inner = src[start + 1 : end]
        stripped = inner.lstrip()
        lead = inner[: len(inner) - len(stripped)]
        m_old = re.match(
            r"(///[^\n]*\n\s*)*(?:resource|name)\s+([A-Za-z_][A-Za-z0-9_]*)\s*;",
            stripped,
        )
        if m_old:
            name = m_old.group(2)
            rest = stripped[m_old.end() :]
            inner = f"{lead}{name} {{{rest}}}"
        inner = rewrite_section_blocks(inner)
        out.append("{")
        out.append(inner)
        out.append("}")
        i = end + 1
    return "".join(out)


def punctuation_pass(src: str) -> str:
    src = re.sub(r"\bprimary\s+true\s*;", "primary;", src)
    src = re.sub(r"\bmin\s*=", "min:", src)
    src = re.sub(r"\bmax\s*=", "max:", src)
    src = re.sub(r"\bdata_layer\s+sqlite\s*;", "store SqliteStore;", src)
    src = re.sub(r"\bdata_layer\s+memory\s*;", "store MemoryStore;", src)
    src = re.sub(r"\bdata_layer\s+postgres\s*;", "store PostgresStore;", src)
    src = re.sub(r"^(\s*)primary\s*$", r"\1primary;", src, flags=re.M)
    src = re.sub(r"^(\s*)authorize_if always\s*$", r"\1authorize_if always;", src, flags=re.M)
    return src


def process_file(path: Path) -> bool:
    original = path.read_text()
    src = original
    src = transform_resource_macros_in_file(src)
    if "quote!" in src and ("resource " in src or "Name {" in src or path.as_posix().endswith("parse.rs") or "validate.rs" in path.as_posix() or "codegen/mod.rs" in path.as_posix() or path.name == "mod.rs"):
        src = transform_quote_headers(src)
    src = punctuation_pass(src)
    if src != original:
        path.write_text(src)
        return True
    return False


def main() -> None:
    changed = 0
    for path in ROOT.rglob("*.rs"):
        if any(part in SKIP_DIRS for part in path.parts):
            continue
        if process_file(path):
            print(f"updated {path.relative_to(ROOT)}")
            changed += 1
    print(f"changed {changed} files")


if __name__ == "__main__":
    main()
