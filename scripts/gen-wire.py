#!/usr/bin/env python3
"""Generate wire crate sources from docs/wire-v0.openrpc.json.

The contract document is the source of truth. After any contract change:
    python3 scripts/gen-wire.py && cargo test -p grid-wire
"""
import json
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CONTRACT = ROOT / "docs" / "wire-v0.openrpc.json"
OUT = ROOT / "crates" / "wire" / "src"

LIB_HEADER = (
    "//! grid-wire - versioned external schemas; the only public vocabulary.\n"
    "//!\n"
    "//! GENERATED from docs/wire-v0.openrpc.json by scripts/gen-wire.py - do not\n"
    "//! edit by hand. deny_unknown_fields everywhere: unknown keys are a wire\n"
    "//! error, never silently dropped.\n"
    "\n"
    "pub mod errors;\n"
    "pub mod methods;\n"
    "\n"
    "use serde::{Deserialize, Serialize};\n"
)
METH_HEADER = (
    "//! Method param and result types, one per wire method. GENERATED.\n"
    "\n"
    "use super::*;\n"
    "use serde::{Deserialize, Serialize};\n"
)

def camel(v):
    return "".join(w.capitalize() for w in re.split(r"[_\-.]", v))


def rust_type(prop, parent, pname, extra):
    if "$ref" in prop:
        return prop["$ref"].split("/")[-1]
    if "oneOf" in prop:
        name = parent + camel(pname)
        extra.append(gen_oneof(name, prop["oneOf"], extra))
        return name
    t = prop.get("type")
    if t == "string":
        if "enum" in prop:
            name = parent + camel(pname)
            extra.append(gen_enum(name, prop["enum"]))
            return name
        return "String"
    if t == "integer":
        return "u64"
    if t == "number":
        return "f64"
    if t == "boolean":
        return "bool"
    if t == "array":
        inner = prop["items"]
        if "$ref" in inner:
            it = inner["$ref"].split("/")[-1]
        elif inner.get("type") == "object":
            name = parent + camel(pname) + "Item"
            extra.append(gen_object(name, inner, extra))
            it = name
        elif inner.get("type") == "string":
            it = "String"
        elif inner.get("type") == "integer":
            it = "u64"
        else:
            raise Exception(f"array item {inner}")
        return f"Vec<{it}>"
    if t == "object":
        name = parent + camel(pname)
        extra.append(gen_object(name, prop, extra))
        return name
    raise Exception(f"prop {parent}.{pname}: {prop}")


def gen_enum(name, values):
    vs = ",\n    ".join(
        f'#[serde(rename = "{v}")]\n    {camel(v)}' for v in values
    )
    return (
        "#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]\n"
        f"pub enum {name} {{\n    {vs},\n}}"
    )


def gen_oneof(name, variants, extra):
    out = []
    for v in variants:
        props = v.get("properties", {})
        keys = list(props.keys())
        vname = (
            camel(keys[0])
            if len(keys) == 1
            else "".join(camel(k) for k in v.get("required", keys))
        )
        fields = []
        for k, p in props.items():
            fields.append((k, rust_type(p, name, k, extra), k in v.get("required", [])))
        out.append((vname, fields))
    body = ""
    for vname, fields in out:
        inner = ", ".join(
            f"{k}: {t}" if req else f"{k}: Option<{t}>" for k, t, req in fields
        )
        body += f"    {vname} {{ {inner} }},\n"
    return (
        "#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]\n"
        f"#[serde(untagged)]\npub enum {name} {{\n{body}}}"
    )


def gen_object(name, v, extra):
    req = set(v.get("required", []))
    lines = []
    for pn, pv in v.get("properties", {}).items():
        t = rust_type(pv, name, pn, extra)
        if pn in req:
            lines.append(f"    pub {pn}: {t},")
        else:
            lines.append(
                "    #[serde(default, skip_serializing_if = \"Option::is_none\")]\n"
                f"    pub {pn}: Option<{t}>,"
            )
    body = "\n".join(lines)
    return (
        "#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]\n"
        f"#[serde(deny_unknown_fields)]\npub struct {name} {{\n{body}\n}}"
    )


def method_mod_name(mname):
    return camel(mname.replace("grid.", "").replace(".", "_"))


def main():
    d = json.load(open(CONTRACT))
    sc = d["components"]["schemas"]
    defs = []
    for n, v in sc.items():
        extra = []
        if "oneOf" in v:
            defs.append(gen_oneof(n, v["oneOf"], extra))
        elif v.get("type") == "object":
            defs.append(gen_object(n, v, extra))
        elif v.get("type") == "string" and "enum" in v:
            defs.append(gen_enum(n, v["enum"]))
        elif v.get("type") == "string":
            defs.append(
                "#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]\n"
                f"#[serde(transparent)]\npub struct {n}(pub String);"
            )
        elif v.get("type") == "array":
            it = v["items"]
            t = it["$ref"].split("/")[-1] if "$ref" in it else "String"
            defs.append(
                "#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]\n"
                f"#[serde(transparent)]\npub struct {n}(pub Vec<{t}>);"
            )
        defs.extend(extra)

    (OUT / "lib.rs").write_text(LIB_HEADER + "\n\n".join(defs) + "\n")

    meth = [METH_HEADER]
    for m in d["methods"]:
        M = method_mod_name(m["name"])
        extra = []
        fields = []
        for p in m["params"]:
            t = rust_type(p["schema"], M, p["name"], extra)
            fields.append((p["name"], t, p.get("required", False)))
        body = "\n".join(
            (
                f"    pub {k}: {t},"
                if r
                else "    #[serde(default, skip_serializing_if = \"Option::is_none\")]\n"
                f"    pub {k}: Option<{t}>,"
            )
            for k, t, r in fields
        )
        meth.append(
            "#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]\n"
            f"#[serde(deny_unknown_fields)]\npub struct {M}Params {{\n{body}\n}}"
        )
        rs = m["result"]["schema"]
        if "$ref" in rs:
            ref = rs["$ref"].split("/")[-1]
            # Skip a self-referential alias (grid.bisect -> BisectResult): the
            # schema type is already in scope via `use super::*`.
            if ref != M + "Result":
                meth.append(f"pub type {M}Result = {ref};")
        elif rs.get("type") == "array":
            it = rs["items"]
            meth.append(f"pub type {M}Result = Vec<{it['$ref'].split('/')[-1]}>;")
        else:
            meth.append(gen_object(M + "Result", rs, extra))
        meth.extend(extra)

    (OUT / "methods.rs").write_text("\n\n".join(meth) + "\n")
    print(f"generated {len(defs)} schema types, {len(d['methods'])} methods")


if __name__ == "__main__":
    main()
