#!/usr/bin/env bash
# Bateria reduzida, exploratória (ADR 0006 §7, revisão pós-benchmark,
# item 4): as cinco tarefas, uma rodada por cenário (A/B/C) — quinze
# rodadas no total. Não substitui a bateria completa (três rodadas por
# cenário); só dá uma direção antes de pagar o custo dela.
#
# A ordem dos cenários roda entre as tarefas (tarefa 01: A,B,C; 02:
# B,C,A; 03: C,A,B; 04: A,B,C; 05: B,C,A) — documentada aqui pra
# registro, mesmo quando as rodadas em si são disparadas em paralelo
# (BENCH_PARALELO, abaixo): cada rodada é um processo `claude -p`
# independente, sem contexto compartilhado entre rodadas, então rodar
# em paralelo não reintroduz o viés de aquecimento que a alternância
# evita numa bateria manual, sequencial.
#
# Uso: bench/bateria-reduzida.sh
# Variáveis de ambiente: as mesmas de bench/rodar.sh, mais
#   BENCH_PARALELO   quantas rodadas simultâneas (default: 4)
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BENCH_PARALELO="${BENCH_PARALELO:-4}"

TAREFAS=(01 02 03 04 05)
ORDENS=(
    "A B C"
    "B C A"
    "C A B"
    "A B C"
    "B C A"
)

if [[ -z "${BENCH_NAS_BIN:-}" ]]; then
    echo "construindo o binário nas fixo (bench/build-nas.sh)..." >&2
    export BENCH_NAS_BIN="$(bash "$REPO_ROOT/bench/build-nas.sh")"
fi
echo "BENCH_NAS_BIN=$BENCH_NAS_BIN" >&2

jobs=0
for i in "${!TAREFAS[@]}"; do
    tarefa="${TAREFAS[$i]}"
    ordem="${ORDENS[$i]}"
    for cenario in $ordem; do
        (
            bash "$REPO_ROOT/bench/rodar.sh" "$tarefa" "$cenario" 1
        ) &
        jobs=$((jobs + 1))
        if (( jobs % BENCH_PARALELO == 0 )); then
            wait -n
        fi
    done
done
wait
echo "bateria reduzida terminada." >&2
