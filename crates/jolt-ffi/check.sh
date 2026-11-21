#!/usr/bin/env bash
# Syntax-checks the csrc shim against Jolt's headers without building Jolt
# itself. Prints diagnostics and nothing else; silent means clean.
#
# The flags must stay in sync with the cc::Build configuration in build.rs:
# Jolt packs its JPH_* defines into a version id and aborts at RegisterTypes()
# if two translation units disagree, so checking against a different define set
# than the real build uses would let a mismatch through.
set -euo pipefail

cd "$(dirname "$0")"

exec "${CXX:-c++}" -fsyntax-only -std=c++17 \
	-DNDEBUG -DJPH_OBJECT_LAYER_BITS=32 \
	-Ivendor/JoltPhysics -Icsrc \
	csrc/*.cpp
