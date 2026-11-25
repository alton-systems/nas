#!/usr/bin/env bash
# Compila o binário `nas` UMA VEZ, a partir de BENCH_COMMIT (o commit
# de partida comum às cinco tarefas), pra servir de servidor MCP fixo
# em todo round do lado "nas" — ADR 0006 §7: a versão do nas sendo
# testada tem que ser a mesma em toda rodada, não recompilada a cada
# vez (e nunca a versão que o próprio agente está editando dentro do
# worktree da rodada, que é só o alvo da edição).
#
# Uso:
#   BENCH_NAS_BIN="$(bench/build-nas.sh)"
#   BENCH_NAS_BIN="$BENCH_NAS_BIN" bench/rodar.sh 01 nas 1
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BENCH_COMMIT="${BENCH_COMMIT:-3bfc9bb}"
BENCH_WORKTREES_DIR="${BENCH_WORKTREES_DIR:-/tmp/nas-bench-worktrees}"
BUILD_DIR="$BENCH_WORKTREES_DIR/build-nas-$BENCH_COMMIT"

mkdir -p "$BENCH_WORKTREES_DIR"
if [[ ! -d "$BUILD_DIR" ]]; then
    git -C "$REPO_ROOT" worktree add -q --detach "$BUILD_DIR" "$BENCH_COMMIT" >&2
fi

cargo build --manifest-path "$BUILD_DIR/Cargo.toml" -p nas-term --bin nas >&2

echo "$BUILD_DIR/target/debug/nas"
