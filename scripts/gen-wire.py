#!/usr/bin/env python3
"""Generate wire crate sources from docs/wire-v1.openrpc.json.

The contract document is the source of truth. After any contract change:
    python3 scripts/gen-wire.py && cargo test -p grid-wire
"""
import json
import re
import textwrap
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CONTRACT = ROOT / "docs" / "wire-v1.openrpc.json"
OUT = ROOT / "crates" / "wire" / "src"

LIB_HEADER = (
    "//! grid-wire - versioned external schemas; the only public vocabulary.\n"
    "//!\n"
    "//! GENERATED from docs/wire-v1.openrpc.json by scripts/gen-wire.py - do not\n"
    "//! edit by hand. `deny_unknown_fields` everywhere: unknown keys are a wire\n"
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
    "use serde::{Deserialize, Serialize};\n"
)
TRAIT = (
    "/// One wire method: the `method` string it is sent under, and the type its\n"
    "/// `result` member decodes into.\n"
    "///\n"
    "/// The params struct is the implementor, so a caller names a method by the\n"
    "/// value it already has to build.\n"
    "pub trait Method: Serialize {\n"
    "    /// The `method` string a request carries.\n"
    "    const NAME: &'static str;\n"
    "    /// What the `result` member decodes into.\n"
    "    type Result: serde::de::DeserializeOwned;\n"
    "}"
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
        # A schema that admits negatives needs a signed Rust type; the
        # contract's `minimum` is what says so.
        return "i64" if prop.get("minimum", 0) < 0 else "u64"
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


PY_OUT = ROOT / "clients" / "python" / "grid_client"
PY_HEADER = (
    '"""GENERATED from docs/wire-v1.openrpc.json by scripts/gen-wire.py - do not edit by hand."""\n'
    "\n"
    "from __future__ import annotations\n"
    "\n"
)


def py_comment(text, indent=""):
    """A schema description as `#` lines. The functional TypedDict form the
    keyword fields `from` and `def` force has no docstring slot."""
    if not text:
        return ""
    return textwrap.fill(text, 100, initial_indent=indent + "# ",
                         subsequent_indent=indent + "# ") + "\n"


def py_literal(values):
    return "Literal[" + ", ".join(json.dumps(v) for v in values) + "]"


def py_type(prop, parent, pname, extra, refs):
    """The mirror of `rust_type`: the same recursion minting the same names for
    the same anonymous schemas, so one schema has one name in both languages.

    Every reference to a generated name is a string, so nothing here depends on
    definition order and a self- or mutually-recursive schema would need no
    change. `refs` is the module prefix a reference carries: empty inside
    schemas.py, `schemas.` from methods.py.
    """
    if "$ref" in prop:
        return f'"{refs}{prop["$ref"].split("/")[-1]}"'
    if "oneOf" in prop:
        name = parent + camel(pname)
        extra.append(py_oneof(name, prop, extra, refs))
        return f'"{name}"'
    t = prop.get("type")
    if t == "string":
        if "enum" in prop:
            name = parent + camel(pname)
            extra.append(py_comment(prop.get("description"))
                         + f"{name}: TypeAlias = {py_literal(prop['enum'])}")
            return f'"{name}"'
        return "str"
    if t == "integer":
        # Python has one integer type, so `rust_type`'s signed/unsigned split on
        # the contract's `minimum` has nothing to decide here.
        return "int"
    if t == "number":
        return "float"
    if t == "boolean":
        return "bool"
    if t == "array":
        inner = prop["items"]
        if "$ref" in inner:
            it = f'"{refs}{inner["$ref"].split("/")[-1]}"'
        elif inner.get("type") == "object":
            name = parent + camel(pname) + "Item"
            extra.append(py_object(name, inner, extra, refs))
            it = f'"{name}"'
        elif inner.get("type") == "string":
            it = "str"
        elif inner.get("type") == "integer":
            it = "int"
        else:
            raise Exception(f"array item {inner}")
        return f"list[{it}]"
    if t == "object":
        name = parent + camel(pname)
        extra.append(py_object(name, prop, extra, refs))
        return f'"{name}"'
    raise Exception(f"prop {parent}.{pname}: {prop}")


def py_object(name, v, extra, refs, parent=None):
    """The functional TypedDict form, not the class form: seven schemas have a
    field named `from` or `def`, which no class body can spell.

    `parent` names the anonymous types a field mints; it is `name` except for a
    method's params, whose fields are named after the method as `rust_type` does.
    """
    req = set(v.get("required", []))
    body = ""
    for pn, pv in v.get("properties", {}).items():
        t = py_type(pv, parent or name, pn, extra, refs)
        body += py_comment(pv.get("description"), "        ")
        body += f'        "{pn}": {t if pn in req else f"NotRequired[{t}]"},\n'
    fields = f"{{\n{body}    }}" if body else "{}"
    return py_comment(v.get("description")) + f'{name} = TypedDict(\n    "{name}",\n    {fields},\n)'


def py_oneof(name, v, extra, refs):
    """A `Union` of one TypedDict per variant. Rust spells these as inline enum
    variants; Python needs a module-level name for each, minted by the same rule
    `gen_oneof` uses for the variant."""
    names = []
    for variant in v["oneOf"]:
        keys = list(variant.get("properties", {}))
        vname = name + (
            camel(keys[0])
            if len(keys) == 1
            else "".join(camel(k) for k in variant.get("required", keys))
        )
        extra.append(py_object(vname, variant, extra, refs))
        names.append(f'"{vname}"')
    return py_comment(v.get("description")) + f"{name}: TypeAlias = Union[{', '.join(names)}]"


def py_schema(n, v):
    extra = []
    if "oneOf" in v:
        top = py_oneof(n, v, extra, "")
    elif v.get("type") == "object":
        top = py_object(n, v, extra, "")
    elif v.get("type") == "string" and "enum" in v:
        top = py_comment(v.get("description")) + f"{n}: TypeAlias = {py_literal(v['enum'])}"
    elif v.get("type") == "string":
        top = py_comment(v.get("description")) + f'{n} = NewType("{n}", str)'
    elif v.get("type") == "array":
        it = v["items"]
        t = f'"{it["$ref"].split("/")[-1]}"' if "$ref" in it else "str"
        top = py_comment(v.get("description")) + f"{n}: TypeAlias = list[{t}]"
    else:
        # The Rust emitter skips a shape it cannot spell; a schema that vanished
        # from one client and not the other is worse than a failed run.
        raise Exception(f"schema {n}: {v}")
    return extra + [top]


def gen_python(d):
    """Write the Python client's generated half.

    Deterministic: contract document order throughout, no `sorted`, no set
    iteration, and both files written with an explicit encoding and newline —
    60 contract descriptions are non-ASCII and a C-locale runner would otherwise
    write different bytes than a UTF-8 one. There is no Python formatter in the
    TCB, so what this emits is what the drift gate compares.
    """
    defs = ["from typing import Literal, NewType, NotRequired, TypeAlias, TypedDict, Union"]
    for n, v in d["components"]["schemas"].items():
        defs.extend(py_schema(n, v))
    (PY_OUT / "schemas.py").write_text(
        PY_HEADER + "\n\n\n".join(defs) + "\n", encoding="utf-8", newline="\n")

    meth = [
        "from collections.abc import Mapping\n"
        "from typing import Literal, NotRequired, TypeAlias, TypedDict, Unpack, cast\n"
        "\n"
        "from . import schemas",
        f'WIRE_VERSION = "{d["info"]["version"]}"',
        "METHODS = (\n" + "".join(f'    "{m["name"]}",\n' for m in d["methods"]) + ")",
        "WIRE_ERRORS: dict[int, str] = {\n"
        + "".join(f'    {e["code"]}: "{n}",\n' for n, e in d["components"]["errors"].items())
        + "}",
    ]
    calls = []
    for m in d["methods"]:
        M = method_mod_name(m["name"])
        extra = []
        params = {
            "properties": {p["name"]: p["schema"] for p in m["params"]},
            "required": [p["name"] for p in m["params"] if p.get("required", False)],
        }
        meth.append(py_object(M + "Params", params, extra, "schemas.", parent=M))
        rs = m["result"]["schema"]
        if "$ref" in rs or rs.get("type") == "array":
            meth.append(f"{M}Result: TypeAlias = {py_type(rs, M, 'result', extra, 'schemas.')}")
        else:
            meth.append(py_object(M + "Result", rs, extra, "schemas."))
        meth.extend(extra)
        snake = m["name"].replace("grid.", "").replace(".", "_")
        calls.append(
            py_comment(m["summary"], "    ")
            + f"    def {snake}(self, **params: Unpack[{M}Params]) -> {M}Result:\n"
            f'        return cast("{M}Result", self.call("{m["name"]}", params))'
        )
    meth.append(
        "class Methods:\n"
        '    """One method per wire method; `call` is the transport a subclass supplies."""\n'
        "\n"
        "    def call(self, method: str, params: Mapping[str, object]) -> object:\n"
        "        raise NotImplementedError\n"
        "\n" + "\n\n".join(calls)
    )
    (PY_OUT / "methods.py").write_text(
        PY_HEADER + "\n\n\n".join(meth) + "\n", encoding="utf-8", newline="\n")


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

    version = (
        "/// The contract's `info.version`. One constant with three consumers, so\n"
        "/// the daemon and its clients cannot disagree about which wire they speak.\n"
        f'pub const WIRE_VERSION: &str = "{d["info"]["version"]}";'
    )
    (OUT / "lib.rs").write_text(
        LIB_HEADER + "\n" + "\n\n".join([version] + defs) + "\n"
    )

    used = set()
    for m in d["methods"]:
        for p in m["params"]:
            for ref in re.findall(r"schemas/(\w+)", json.dumps(p["schema"])):
                used.add(ref)
        # A result that *is* a schema type (grid.bisect -> BisectResult) emits
        # no alias below, so importing it would be an unused import.
        own = method_mod_name(m["name"]) + "Result"
        for ref in re.findall(r"schemas/(\w+)", json.dumps(m["result"]["schema"])):
            if ref != own:
                used.add(ref)
    meth = [METH_HEADER, f"use super::{{{', '.join(sorted(used))}}};\n", TRAIT]
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
        result = f"{M}Result"
        if "$ref" in rs:
            ref = rs["$ref"].split("/")[-1]
            if ref == M + "Result":
                # A self-referential alias (grid.bisect -> BisectResult) is not
                # emitted and the schema type is not imported either, so the impl
                # below is the only reader and it reaches through `super`.
                result = f"super::{ref}"
            else:
                meth.append(f"pub type {M}Result = {ref};")
        elif rs.get("type") == "array":
            it = rs["items"]
            meth.append(f"pub type {M}Result = Vec<{it['$ref'].split('/')[-1]}>;")
        else:
            meth.append(gen_object(M + "Result", rs, extra))
        meth.extend(extra)
        meth.append(
            f"impl Method for {M}Params {{\n"
            f'    const NAME: &\'static str = "{m["name"]}";\n'
            f"    type Result = {result};\n"
            "}"
        )

    names = ",\n    ".join(f'"{m["name"]}"' for m in d["methods"])
    meth.append(
        "/// Every method the contract declares, in contract order.\n"
        f"pub const METHODS: [&str; {len(d['methods'])}] = [\n    {names},\n];"
    )
    (OUT / "methods.rs").write_text("\n\n".join(meth) + "\n")
    gen_python(d)
    print(f"generated {len(defs)} schema types, {len(d['methods'])} methods")


if __name__ == "__main__":
    main()
