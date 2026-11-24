"""Translate partial Web IDL interfaces containing promise-returning operations to TypeScript."""

import argparse
from pathlib import Path
import re


TYPES = {"ArrayBuffer": "ArrayBuffer", "DOMString": "string", "undefined": "void",
         "boolean": "boolean", "USVString": "string"}


def typescript_type(name):
    try:
        return TYPES[name]
    except KeyError:
        raise ValueError(f"Unsupported Web IDL type {name}") from None


def derive(source):
    source = re.sub(r"//[^\n]*", "", source).strip()
    declarations = []
    while source:
        interface = re.match(r"partial\s+interface\s+(\w+)\s*\{([^{}]*)\}\s*;", source)
        if not interface:
            raise ValueError(f"Expected a partial interface near {source[:60]!r}")
        name, body = interface.groups()
        declarations.append(f"export interface {name}Binding {{")
        body = body.strip()
        while body:
            method = re.match(r"Promise\s*<\s*(\w+)\s*>\s+(\w+)\s*\(([^()]*)\)\s*;", body)
            if not method:
                raise ValueError(f"Unsupported operation in {name} near {body[:60]!r}")
            result, operation, parameters = method.groups()
            arguments = []
            if parameters.strip():
                for parameter in parameters.split(","):
                    arg_type, arg_name = parameter.split()
                    arguments.append(f"{arg_name}: {typescript_type(arg_type)}")
            declarations.append(f"    {operation}({', '.join(arguments)}): Promise<{typescript_type(result)}>;")
            body = body[method.end():].strip()
        declarations.append("}")
        source = source[interface.end():].strip()
    return "\n".join(declarations) + "\n"


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("idl", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(derive(args.idl.read_text(encoding="utf-8")), encoding="utf-8")
