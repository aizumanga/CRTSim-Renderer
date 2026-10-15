#!/bin/sh
# CC0. Builds `check` from ReShade's FX compiler, as vkBasalt's Ubuntu source package carries
# it (ReShade's own code, BSD-3-Clause): `apt-get source vkbasalt` with source packages
# enabled, and the spirv-headers package. Usage: build.sh VKBASALT_SOURCE_DIR OUTPUT
set -eu
src="$1/src/reshade"
here="$(dirname "$0")"
c++ -std=c++17 -O1 -w -I"$src" -o "$2" "$here/check.cpp" \
    "$src/effect_codegen_spirv.cpp" "$src/effect_expression.cpp" "$src/effect_lexer.cpp" \
    "$src/effect_parser.cpp" "$src/effect_preprocessor.cpp" "$src/effect_symbol_table.cpp"
