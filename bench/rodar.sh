#!/usr/bin/env bash
# Roda uma rodada do benchmark comparativo (ADR 0006 §7, etapa 3f).
#
# Uso:
#   bench/rodar.sh <tarefa> <lado> <rodada>
#
#   tarefa  — "01".."05" (o prefixo do arquivo em bench/tarefas/)
#   lado    — "normal" (ferramentas próprias do Claude Code) ou
#             "nas" (só as ferramentas do nas-mcp, Read/Edit/Write/
#             MultiEdit/NotebookEdit desligadas)
#   rodada  — um número (1, 2, 3, ...) ou "ensaio" (pra validar o
#             script sem contar no resultado — ADR 0006 §7, passo 1
#             da etapa 3f-prep)
#
# Variáveis de ambiente:
#   BENCH_MODEL           modelo fixo nos dois lados (default: claude-sonnet-5)
#   BENCH_COMMIT          commit de partida (default: 3bfc9bb — o topo
#                         da main quando as tarefas foram escolhidas)
#   BENCH_WORKTREES_DIR   onde criar os worktrees efêmeros (default:
#                         /tmp/nas-bench-worktrees — fora do próprio
#                         repositório, nunca commitados)
#   BENCH_NAS_BIN         caminho do binário `nas` já compilado, usado
#                         como servidor MCP no lado "nas" — construído
#                         UMA VEZ a partir de BENCH_COMMIT (ver
#                         `bench/build-nas.sh`), não a cada rodada: é a
#                         versão fixa do nas sendo testada, igual pra
#                         toda rodada de todo lado "nas". Obrigatório
#                         se lado == nas.
#
# Cada rodada:
#   1. cria um worktree Git limpo em BENCH_COMMIT;
#   2. no lado "nas", grava permissions.toml (bench/permissoes-benchmark.toml)
#      no diretório de estado daquele worktree, e monta um
#      --mcp-config apontando pra BENCH_NAS_BIN;
#   3. chama `claude -p` com o texto de bench/tarefas/<tarefa>-*.md,
#      --output-format stream-json (transcrição completa) e
#      --model fixo;
#   4. salva a transcrição e o resumo em
#      bench/resultados/<tarefa>/<lado>/<rodada>/;
#   5. roda `cargo test --workspace` + a verificação específica da
#      tarefa dentro do worktree, grava o resultado em verification.json;
#   6. remove o worktree (o resultado já foi copiado pra fora dele).
set -euo pipefail

if [[ $# -ne 3 ]]; then
    echo "uso: $0 <tarefa 01-05> <lado normal|nas> <rodada N|ensaio>" >&2
    exit 1
fi

TAREFA="$1"
LADO="$2"
RODADA="$3"

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BENCH_MODEL="${BENCH_MODEL:-claude-sonnet-5}"
BENCH_COMMIT="${BENCH_COMMIT:-3bfc9bb}"
BENCH_WORKTREES_DIR="${BENCH_WORKTREES_DIR:-/tmp/nas-bench-worktrees}"

if [[ "$LADO" != "normal" && "$LADO" != "nas" ]]; then
    echo "lado inválido: $LADO (use 'normal' ou 'nas')" >&2
    exit 1
fi

TAREFA_ARQUIVO="$(ls "$REPO_ROOT"/bench/tarefas/"$TAREFA"-*.md 2>/dev/null | head -1)"
if [[ -z "$TAREFA_ARQUIVO" ]]; then
    echo "nenhuma tarefa encontrada com prefixo '$TAREFA' em bench/tarefas/" >&2
    exit 1
fi

if [[ "$LADO" == "nas" && -z "${BENCH_NAS_BIN:-}" ]]; then
    echo "BENCH_NAS_BIN precisa apontar pro binário nas já compilado (ver bench/build-nas.sh)" >&2
    exit 1
fi

RESULT_DIR="$REPO_ROOT/bench/resultados/$TAREFA/$LADO/$RODADA"
mkdir -p "$RESULT_DIR"

WORKTREE="$BENCH_WORKTREES_DIR/$TAREFA-$LADO-$RODADA-$$"
mkdir -p "$BENCH_WORKTREES_DIR"
git -C "$REPO_ROOT" worktree add -q --detach "$WORKTREE" "$BENCH_COMMIT"
cleanup() {
    git -C "$REPO_ROOT" worktree remove --force "$WORKTREE" >/dev/null 2>&1 || true
}
trap cleanup EXIT

# Extrai só o bloco "## Texto exato do pedido" do arquivo da tarefa —
# entre essa linha e a próxima linha "## " — e tira o "> " de citação
# Markdown de cada linha, pra sobrar o texto puro do pedido.
PROMPT_FILE="$RESULT_DIR/prompt.txt"
awk '
    /^## Texto exato do pedido$/ { capturando=1; next }
    capturando && /^## / { exit }
    capturando { print }
' "$TAREFA_ARQUIVO" | sed -E 's/^> ?//' | sed '/^\s*$/N;/\n\s*$/D' > "$PROMPT_FILE"

echo "== tarefa $TAREFA, lado $LADO, rodada $RODADA ==" >&2
echo "worktree: $WORKTREE" >&2

# Aviso igual nos dois lados (pedido explícito, registrado em
# docs/benchmark-fase3.md §4): numa rodada automatizada não há
# ninguém pra aprovar nada no meio do caminho — sem isto, um lado
# pode parar mais cedo "esperando revisão" (como o lado nas fez no
# ensaio, por ter lido o CLAUDE.md do próprio worktree) e o outro não,
# o que mediria estilo de parada, não a tarefa em si.
AVISO_BENCHMARK="Execução automatizada de benchmark: não há ninguém para aprovar. Conclua a tarefa por completo, sem pedir confirmação. Não faça commit nem push."

COMMON_ARGS=(
    --output-format stream-json
    --verbose
    --include-partial-messages
    --model "$BENCH_MODEL"
    --strict-mcp-config
    --append-system-prompt "$AVISO_BENCHMARK"
)

# Shell liberado igual nos dois lados: só cargo e git (ADR 0006 §7 —
# "mesmas permissões nos dois lados, documentadas"). As variantes
# "rtk cargo"/"rtk git" cobrem o hook local de RTK (proxy de
# ferramenta deste usuário, ~/.claude/RTK.md), que pode reescrever um
# comando por baixo antes da checagem de permissão chegar a vê-lo —
# sem as duas formas, uma rodada poderia travar numa aprovação de
# shell por causa de um hook que nada tem a ver com nas vs. normal.
# Afeta os dois lados igualmente (mesma configuração de usuário), por
# isso não é tratado como viés entre os lados — só registrado aqui.
SHELL_ALLOW=(
    "Bash(cargo *)" "Bash(rtk cargo *)"
    "Bash(git *)" "Bash(rtk git *)"
    Glob Grep
)

TRANSCRIPT="$RESULT_DIR/transcript.jsonl"
START_TS=$(date +%s)

if [[ "$LADO" == "normal" ]]; then
    (
        cd "$WORKTREE"
        claude -p "$(cat "$PROMPT_FILE")" \
            "${COMMON_ARGS[@]}" \
            --permission-mode acceptEdits \
            --allowedTools "Read" "Edit" "Write" "MultiEdit" "${SHELL_ALLOW[@]}"
    ) > "$TRANSCRIPT"
else
    MCP_CONFIG="$RESULT_DIR/mcp-config.json"
    python3 -c "
import json, sys
json.dump({'mcpServers': {'nas': {'command': sys.argv[1], 'args': ['--mcp']}}}, open(sys.argv[2], 'w'))
" "$BENCH_NAS_BIN" "$MCP_CONFIG"

    # `nas --permissions`, rodado uma vez dentro do worktree (a raiz
    # do projeto é descoberta pelo diretório atual — ADR 0005 §6),
    # cria o permissions.toml no lugar certo; depois substituímos o
    # modelo "tudo negado" pela política do benchmark.
    PERMISSIONS_PATH="$(cd "$WORKTREE" && "$BENCH_NAS_BIN" --permissions | sed -n 's/^Arquivo de política: //p')"
    cp "$REPO_ROOT/bench/permissoes-benchmark.toml" "$PERMISSIONS_PATH"

    (
        cd "$WORKTREE"
        claude -p "$(cat "$PROMPT_FILE")" \
            "${COMMON_ARGS[@]}" \
            --mcp-config "$MCP_CONFIG" \
            --disallowedTools "Read" "Edit" "Write" "MultiEdit" "NotebookEdit" \
            --allowedTools "mcp__nas__*" "${SHELL_ALLOW[@]}"
    ) > "$TRANSCRIPT"
fi

END_TS=$(date +%s)
DURATION=$((END_TS - START_TS))

# A última linha do stream-json é o resumo (`"type":"result"`) — o
# mesmo conteúdo que `--output-format json` devolveria sozinho.
tail -1 "$TRANSCRIPT" > "$RESULT_DIR/result.json"

python3 "$REPO_ROOT/bench/verificar.py" "$TAREFA" "$WORKTREE" "$DURATION" > "$RESULT_DIR/verification.json"

cat <<EOF > "$RESULT_DIR/meta.json"
{
  "tarefa": "$TAREFA",
  "lado": "$LADO",
  "rodada": "$RODADA",
  "modelo": "$BENCH_MODEL",
  "commit_de_partida": "$BENCH_COMMIT",
  "duracao_parede_s": $DURATION
}
EOF

echo "resultado em: $RESULT_DIR" >&2
cat "$RESULT_DIR/verification.json" >&2
