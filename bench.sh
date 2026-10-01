#!/usr/bin/env bash
# Usage : ./bench.sh [n] [config]   (défaut : 1_000_000_000, config 0)
# Teste plusieurs tailles de segment (2^17 .. 2^22 bits), 3 essais chacune.
N="${1:-1_000_000_000}"
CFG="${2:-0}"
BIN="./target/release/burn_prems"
for e in 15 16 17 18 19 20; do
  for essai in 1 2 3; do
    SEG_LOG2=$e "$BIN" "$N" "$CFG"
  done
done
