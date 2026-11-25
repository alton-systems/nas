#!/usr/bin/env python3
"""Verificação objetiva de uma rodada do benchmark (ADR 0006 §7).

Roda `cargo test --workspace` dentro do worktree da rodada, mais a
checagem específica da tarefa (espelha a seção "Verificação objetiva"
do arquivo correspondente em bench/tarefas/ — se um arquivo de tarefa
mudar, esta função precisa mudar junto). Imprime um JSON em stdout com
o resultado; nunca lança excursão silenciosa — qualquer passo que
falhe aparece em "detalhes".

Uso: verificar.py <tarefa 01-05> <diretório do worktree> <duracao_s>
"""

import json
import re
import subprocess
import sys
from pathlib import Path


def rodar(args, cwd):
    return subprocess.run(
        args, cwd=cwd, capture_output=True, text=True, timeout=600
    )


def cargo_test_ok(worktree):
    r = rodar(["cargo", "test", "--workspace"], worktree)
    return r.returncode == 0, (r.stdout[-4000:] + r.stderr[-4000:])


def check_01(worktree):
    # nenhuma das duas funções chama nas_history::list_checkpoints, e
    # existe um teste novo (heurística: nome de teste mencionando
    # "poda" ou "prune" perto de "sessao"/"session") — em QUALQUER
    # arquivo do crate, não só registry.rs: um teste de ponta a ponta
    # pra isto plausivelmente mora em commands.rs, ao lado dos outros
    # testes de checkpoint de sessão (ensaio da etapa 3f-prep PASSO 1
    # pegou esse caso real: a implementação estava certa, só a
    # verificação é que procurava no arquivo errado).
    texto = Path(worktree, "crates/nas-commands/src/registry.rs").read_text()
    m = re.search(
        r"fn prepare_session_checkpoint.*?\n\}\n.*?fn session_checkpoint_exists.*?\n\}",
        texto,
        re.S,
    )
    bloco = m.group(0) if m else texto
    sem_list_checkpoints = "list_checkpoints" not in bloco

    texto_crate = "\n".join(
        p.read_text() for p in Path(worktree, "crates/nas-commands/src").glob("*.rs")
    )
    tem_teste_novo = bool(
        re.search(r"fn \w*(poda|prune)\w*sessao\w*|fn \w*sessao\w*(poda|prune)\w*", texto_crate)
    )
    return sem_list_checkpoints and tem_teste_novo, {
        "sem_list_checkpoints_no_bloco": sem_list_checkpoints,
        "teste_novo_encontrado_por_heuristica": tem_teste_novo,
        "nota": "heurística de nome de teste pode falhar num nome válido "
        "mas diferente — revisar à mão se tem_teste_novo vier falso",
    }


def check_02(worktree):
    texto = Path(worktree, "crates/nas-syntax/src/markdown.rs").read_text()
    m = re.search(r"mod tests \{.*", texto, re.S)
    bloco = m.group(0) if m else ""
    chamadas_diretas = len(re.findall(r"parse_atx_heading\(", bloco))
    return chamadas_diretas >= 1, {
        "chamadas_diretas_em_mod_tests": chamadas_diretas,
        "nota": "cobertura dos 6 casos do pedido precisa de revisão manual",
    }


def check_03(worktree):
    texto = Path(worktree, "crates/nas-commands/src/commands.rs").read_text()
    restos = re.findall(r"\bfrom_line\b|\bto_line\b", texto)
    return len(restos) == 0, {"ocorrencias_restantes": len(restos)}


def check_04(worktree):
    saida = subprocess.run(
        ["grep", "-rn", "current_content_for(&entry.path)", "crates/nas-commands/src/"],
        cwd=worktree,
        capture_output=True,
        text=True,
    ).stdout
    ocorrencias = len([l for l in saida.splitlines() if l.strip()])
    return ocorrencias == 1, {"ocorrencias": ocorrencias}


def check_05(worktree):
    # Categoria "explicar" — sem checagem mecânica; a nota vem do
    # critério escrito em bench/tarefas/05-... contra a resposta em
    # texto, lida do transcript.jsonl depois.
    return None, {"nota": "avaliação manual contra o critério de 5 pontos do próprio arquivo da tarefa"}


CHECKS = {
    "01": check_01,
    "02": check_02,
    "03": check_03,
    "04": check_04,
    "05": check_05,
}


def main():
    tarefa, worktree, duracao_s = sys.argv[1], sys.argv[2], sys.argv[3]
    worktree = Path(worktree)

    cargo_ok, cargo_detalhes = cargo_test_ok(worktree)
    tarefa_ok, tarefa_detalhes = CHECKS[tarefa](worktree)

    if tarefa_ok is None:
        sucesso = None
    else:
        sucesso = cargo_ok and tarefa_ok

    print(
        json.dumps(
            {
                "tarefa": tarefa,
                "duracao_s": duracao_s,
                "cargo_test_ok": cargo_ok,
                "tarefa_check_ok": tarefa_ok,
                "sucesso": sucesso,
                "cargo_test_detalhes": cargo_detalhes,
                "tarefa_detalhes": tarefa_detalhes,
            },
            ensure_ascii=False,
            indent=2,
        )
    )


if __name__ == "__main__":
    main()
