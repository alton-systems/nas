//! `nas --mcp` roda o binário de verdade sobre stdio — só um
//! processo real exercita o protocolo ponta a ponta (etapa 3c, ADR
//! 0006-ia-trabalha-pelo-nas.md §3): initialize, tools/list,
//! tools/call, e a garantia de que nada além do JSON-RPC aparece na
//! stdout.

use std::collections::BTreeSet;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use serde_json::{Value, json};

struct McpProcess {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: i64,
}

impl McpProcess {
    fn spawn(project_dir: &Path, state_home: &Path, approve_writes: bool) -> Self {
        let mut command = Command::new(env!("CARGO_BIN_EXE_nas"));
        command
            .arg("--mcp")
            .current_dir(project_dir)
            .env("XDG_STATE_HOME", state_home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if approve_writes {
            command.arg("--approve-writes");
        }
        let mut child = command.spawn().expect("iniciar nas --mcp");
        let stdin = child.stdin.take().expect("stdin do processo");
        let stdout = BufReader::new(child.stdout.take().expect("stdout do processo"));
        McpProcess {
            child,
            stdin,
            stdout,
            next_id: 1,
        }
    }

    fn send(&mut self, value: &Value) {
        let mut line = serde_json::to_string(value).expect("serializar requisição");
        line.push('\n');
        self.stdin
            .write_all(line.as_bytes())
            .expect("escrever no stdin do processo");
        self.stdin.flush().expect("flush do stdin");
    }

    /// Lê uma linha da stdout e confirma que é um JSON-RPC válido —
    /// é também a verificação de "stdout limpo" (nada além do
    /// protocolo), já que qualquer outra coisa ali faria o parse
    /// falhar.
    fn recv_line(&mut self) -> Value {
        let mut line = String::new();
        self.stdout
            .read_line(&mut line)
            .expect("ler linha da stdout");
        assert!(
            !line.is_empty(),
            "stdout fechou antes de responder — stderr: {}",
            self.drain_stderr()
        );
        let value: Value = serde_json::from_str(&line).unwrap_or_else(|err| {
            panic!("linha da stdout não é JSON-RPC válido: {line:?} ({err})")
        });
        assert_eq!(value["jsonrpc"], "2.0", "linha sem jsonrpc=2.0: {line:?}");
        value
    }

    fn drain_stderr(&mut self) -> String {
        use std::io::Read as _;
        let mut stderr = String::new();
        if let Some(mut handle) = self.child.stderr.take() {
            let _ = handle.read_to_string(&mut stderr);
        }
        stderr
    }

    fn initialize(&mut self, client_name: &str) {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": client_name, "version": "0.0.1"}
            }
        }));
        let response = self.recv_line();
        assert_eq!(response["id"], id, "resposta do initialize: {response}");
        self.send(&json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
            "params": {}
        }));
    }

    fn list_tools(&mut self) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": "tools/list", "params": {}}));
        self.recv_line()["result"].clone()
    }

    fn call_tool(&mut self, name: &str, arguments: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": {"name": name, "arguments": arguments}
        }));
        self.recv_line()["result"].clone()
    }

    /// Texto da primeira entrada de conteúdo de um `tools/call` —
    /// o sucesso serializa o `serde_json::Value` do comando como
    /// texto; o erro, a mensagem de `CommandError`.
    fn call_tool_text(&mut self, name: &str, arguments: Value) -> (bool, String) {
        let result = self.call_tool(name, arguments);
        let is_error = result["isError"].as_bool().unwrap_or(false);
        let text = result["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        (is_error, text)
    }

    /// O texto de cada bloco de conteúdo, em ordem — pra ferramentas
    /// que devolvem texto puro (`read`/`search`/`list_files`/
    /// `show_diff`), onde um aviso vira um segundo bloco, nunca
    /// misturado ao primeiro.
    fn call_tool_content_texts(&mut self, name: &str, arguments: Value) -> Vec<String> {
        let result = self.call_tool(name, arguments);
        result["content"]
            .as_array()
            .expect("content é uma lista")
            .iter()
            .map(|block| block["text"].as_str().unwrap_or_default().to_string())
            .collect()
    }

    fn kill(mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Mesma identificação de projeto que `nas --mcp` usa
/// (`discover_project`/`ProjectId::discover`) — pra achar, de fora,
/// a mesma pasta de histórico que o processo escreveu.
fn history_dir_for(project_dir: &Path, state_home: &Path) -> PathBuf {
    let canonical = project_dir.canonicalize().expect("canonicalizar projeto");
    let project = nas_history::ProjectId::discover(&canonical).expect("descobrir projeto");
    project
        .ensure_history_dir(state_home)
        .expect("pasta de histórico")
}

fn exposed_tool_names() -> BTreeSet<String> {
    let mut registry = nas_commands::Registry::new();
    nas_commands::register_all(&mut registry);
    registry
        .specs()
        .iter()
        .filter(|spec| spec.exposed_to_ai)
        .map(|spec| spec.name.to_string())
        .collect()
}

#[test]
fn lista_de_ferramentas_e_igual_ao_registro_filtrado() {
    let project_dir = tempfile::tempdir().expect("diretório do projeto");
    let state_home = tempfile::tempdir().expect("diretório de estado");

    let mut mcp = McpProcess::spawn(project_dir.path(), state_home.path(), false);
    mcp.initialize("teste-lista");
    let tools = mcp.list_tools()["tools"].clone();

    let names: BTreeSet<String> = tools
        .as_array()
        .expect("tools é uma lista")
        .iter()
        .map(|tool| tool["name"].as_str().expect("tool tem nome").to_string())
        .collect();

    assert_eq!(names, exposed_tool_names());
    mcp.kill();
}

#[test]
fn edit_via_adaptador_muda_o_buffer_e_journala_como_ai() {
    let project_dir = tempfile::tempdir().expect("diretório do projeto");
    let state_home = tempfile::tempdir().expect("diretório de estado");
    std::fs::write(project_dir.path().join("foo.txt"), "ola\n").expect("escrever foo.txt");

    let mut mcp = McpProcess::spawn(project_dir.path(), state_home.path(), true);
    mcp.initialize("agente-teste");

    let (is_error, _) = mcp.call_tool_text("open_file", json!({"path": "foo.txt"}));
    assert!(!is_error, "open_file falhou");

    let (is_error, _) = mcp.call_tool_text(
        "edit",
        json!({"path": "foo.txt", "old_text": "ola", "new_text": "oi"}),
    );
    assert!(!is_error, "edit falhou");

    let (is_error, _) = mcp.call_tool_text("save_file", json!({"path": "foo.txt"}));
    assert!(!is_error, "save_file falhou mesmo com --approve-writes");

    let disk_content =
        std::fs::read_to_string(project_dir.path().join("foo.txt")).expect("ler foo.txt do disco");
    assert_eq!(disk_content, "oi\n");

    let history_dir = history_dir_for(project_dir.path(), state_home.path());
    let canonical_project = project_dir.path().canonicalize().unwrap();
    let expected_file = canonical_project
        .join("foo.txt")
        .to_string_lossy()
        .into_owned();
    let entries =
        nas_history::file_history(&history_dir, &expected_file).expect("entradas não encontradas");
    let edit_entry = entries
        .iter()
        .find(|entry| entry.command == "edit")
        .unwrap_or_else(|| panic!("nenhuma entrada 'edit' no journal: {entries:?}"));
    assert_eq!(
        edit_entry.author,
        nas_history::Author::Ai {
            agent: "agente-teste".to_string(),
            session: entry_session(edit_entry),
        }
    );

    mcp.kill();
}

fn entry_session(entry: &nas_history::JournalEntry) -> String {
    match &entry.author {
        nas_history::Author::Ai { session, .. } => session.clone(),
        other => panic!("autor não é Ai: {other:?}"),
    }
}

#[test]
fn journal_sobrevive_a_um_encerramento_a_forca_depois_de_cada_chamada() {
    let project_dir = tempfile::tempdir().expect("diretório do projeto");
    let state_home = tempfile::tempdir().expect("diretório de estado");
    std::fs::write(project_dir.path().join("foo.txt"), "ola mundo\n").expect("escrever foo.txt");

    let mut mcp = McpProcess::spawn(project_dir.path(), state_home.path(), false);
    mcp.initialize("agente-forca-bruta");

    mcp.call_tool_text("open_file", json!({"path": "foo.txt"}));
    mcp.call_tool_text(
        "edit",
        json!({"path": "foo.txt", "old_text": "ola", "new_text": "primeira"}),
    );
    // Duas chamadas em sequência rápida, bem dentro da mesma janela
    // de lote do journal (`BATCH_INTERVAL_SECONDS`) — sem o flush
    // incondicional ao fim de cada chamada (`Registry::run`), a
    // entrada desta segunda edição ficaria só na fila em memória,
    // vulnerável a um `kill -9` como o de `mcp.kill()` abaixo (nunca
    // roda `Drop`, que é só melhor esforço mesmo quando roda).
    let (is_error, _) = mcp.call_tool_text(
        "edit",
        json!({"path": "foo.txt", "old_text": "mundo", "new_text": "segunda"}),
    );
    assert!(!is_error, "a segunda edição devia funcionar");

    // `kill`, não um encerramento gracioso — é exatamente o cenário
    // que preocupa: o processo nunca chega a rodar `Drop::drop` do
    // `Journal`.
    mcp.kill();

    let history_dir = history_dir_for(project_dir.path(), state_home.path());
    let canonical_project = project_dir.path().canonicalize().unwrap();
    let expected_file = canonical_project
        .join("foo.txt")
        .to_string_lossy()
        .into_owned();
    let entries =
        nas_history::file_history(&history_dir, &expected_file).expect("entradas não encontradas");
    let commands: Vec<&str> = entries.iter().map(|e| e.command.as_str()).collect();
    assert_eq!(
        commands,
        vec!["edit", "edit"],
        "as duas edições deviam estar no journal em disco, não só em memória: {entries:?}"
    );
}

#[test]
fn save_file_recusado_sem_approve_writes() {
    let project_dir = tempfile::tempdir().expect("diretório do projeto");
    let state_home = tempfile::tempdir().expect("diretório de estado");
    std::fs::write(project_dir.path().join("foo.txt"), "ola\n").expect("escrever foo.txt");

    let mut mcp = McpProcess::spawn(project_dir.path(), state_home.path(), false);
    mcp.initialize("teste-sem-flag");

    mcp.call_tool_text("open_file", json!({"path": "foo.txt"}));
    mcp.call_tool_text(
        "edit",
        json!({"path": "foo.txt", "old_text": "ola", "new_text": "oi"}),
    );
    let (is_error, text) = mcp.call_tool_text("save_file", json!({"path": "foo.txt"}));
    assert!(is_error, "save_file deveria recusar sem --approve-writes");
    assert!(
        text.contains("3d") || text.to_lowercase().contains("aprova"),
        "mensagem de recusa não menciona aprovação: {text}"
    );

    let disk_content =
        std::fs::read_to_string(project_dir.path().join("foo.txt")).expect("ler foo.txt do disco");
    assert_eq!(
        disk_content, "ola\n",
        "save_file não deveria ter escrito nada"
    );

    mcp.kill();
}

#[test]
fn save_file_recusa_quando_o_disco_mudou() {
    let project_dir = tempfile::tempdir().expect("diretório do projeto");
    let state_home = tempfile::tempdir().expect("diretório de estado");
    let file_path = project_dir.path().join("foo.txt");
    std::fs::write(&file_path, "ola\n").expect("escrever foo.txt");

    let mut mcp = McpProcess::spawn(project_dir.path(), state_home.path(), true);
    mcp.initialize("teste-disco-mudou");

    mcp.call_tool_text("open_file", json!({"path": "foo.txt"}));
    mcp.call_tool_text(
        "edit",
        json!({"path": "foo.txt", "old_text": "ola", "new_text": "oi"}),
    );

    // Mudança externa concorrente, depois da leitura da IA — um
    // outro processo (ou a pessoa, direto no editor) escrevendo o
    // mesmo arquivo.
    std::fs::write(&file_path, "mudou por fora\n").expect("simular mudança externa");

    let (is_error, text) = mcp.call_tool_text("save_file", json!({"path": "foo.txt"}));
    assert!(
        is_error,
        "save_file deveria recusar quando o disco mudou: {text}"
    );
    assert!(
        text.contains("open_file") || text.to_lowercase().contains("mudou"),
        "mensagem de recusa não menciona o disco mudado: {text}"
    );

    let disk_content = std::fs::read_to_string(&file_path).expect("ler foo.txt do disco");
    assert_eq!(
        disk_content, "mudou por fora\n",
        "save_file nunca deve sobrescrever trabalho de outro processo"
    );

    // De novo, sem nenhuma escrita nova no meio: continua recusando —
    // a primeira tentativa não pode ter "resolvido" a pendência só
    // por ter conferido o disco.
    let (is_error, text) = mcp.call_tool_text("save_file", json!({"path": "foo.txt"}));
    assert!(
        is_error,
        "save_file deveria recusar de novo, sem nenhuma escrita nova no meio: {text}"
    );
    let disk_content = std::fs::read_to_string(&file_path).expect("ler foo.txt do disco");
    assert_eq!(
        disk_content, "mudou por fora\n",
        "a segunda tentativa também não pode sobrescrever a mudança externa"
    );

    // `open_file` é a saída que a mensagem de erro indica: traz o
    // conteúdo do disco, descartando a edição não salva da IA, e diz
    // isso no resultado.
    let (is_error, text) = mcp.call_tool_text("open_file", json!({"path": "foo.txt"}));
    assert!(!is_error, "reabrir devia funcionar: {text}");
    assert!(
        text.to_lowercase().contains("descart"),
        "reabrir devia avisar que descartou a edição não salva: {text}"
    );
    let read_texts = mcp.call_tool_content_texts("read", json!({"path": "foo.txt"}));
    assert_eq!(read_texts, vec!["1 mudou por fora".to_string()]);

    // Com o buffer resincronizado, editar e salvar funcionam — e
    // preservam o que o outro processo escreveu.
    let (is_error, text) = mcp.call_tool_text(
        "edit",
        json!({
            "path": "foo.txt",
            "old_text": "mudou por fora",
            "new_text": "mudou por fora, e a IA editou por cima"
        }),
    );
    assert!(
        !is_error,
        "editar devia funcionar depois de reabrir: {text}"
    );
    let (is_error, text) = mcp.call_tool_text("save_file", json!({"path": "foo.txt"}));
    assert!(
        !is_error,
        "salvar devia funcionar depois de reabrir e editar: {text}"
    );

    let disk_content = std::fs::read_to_string(&file_path).expect("ler foo.txt do disco");
    assert_eq!(
        disk_content, "mudou por fora, e a IA editou por cima\n",
        "o conteúdo do outro processo devia continuar na base do arquivo final"
    );

    mcp.kill();
}

#[test]
fn buffer_limpo_recarrega_do_disco_antes_de_ler() {
    let project_dir = tempfile::tempdir().expect("diretório do projeto");
    let state_home = tempfile::tempdir().expect("diretório de estado");
    let file_path = project_dir.path().join("foo.txt");
    std::fs::write(&file_path, "ola\n").expect("escrever foo.txt");

    let mut mcp = McpProcess::spawn(project_dir.path(), state_home.path(), false);
    mcp.initialize("teste-recarrega");

    mcp.call_tool_text("open_file", json!({"path": "foo.txt"}));

    // Nenhuma edição da IA ainda — o buffer está limpo; uma mudança
    // externa deve recarregar sozinha, sem aviso nenhum.
    std::fs::write(&file_path, "mudou por fora\n").expect("simular mudança externa");

    let texts = mcp.call_tool_content_texts("read", json!({"path": "foo.txt"}));
    assert_eq!(
        texts,
        vec!["1 mudou por fora".to_string()],
        "buffer limpo não deveria avisar (um bloco só), e o texto já deveria vir do disco recarregado"
    );

    mcp.kill();
}

#[test]
fn leituras_devolvem_texto_puro_e_aviso_num_segundo_bloco() {
    let project_dir = tempfile::tempdir().expect("diretório do projeto");
    let state_home = tempfile::tempdir().expect("diretório de estado");
    let file_path = project_dir.path().join("foo.txt");
    std::fs::write(&file_path, "linha um\nlinha dois\nlinha tres\n").expect("escrever foo.txt");

    let mut mcp = McpProcess::spawn(project_dir.path(), state_home.path(), false);
    mcp.initialize("teste-texto-puro");

    mcp.call_tool_text("open_file", json!({"path": "foo.txt"}));

    // `read`/`search`/`list_files` devolvem texto puro, linhas
    // numeradas quando fizer sentido — nunca JSON com `\n` escapado.
    let read_texts = mcp.call_tool_content_texts("read", json!({"path": "foo.txt"}));
    assert_eq!(
        read_texts,
        vec!["1 linha um\n2 linha dois\n3 linha tres".to_string()]
    );

    let search_texts =
        mcp.call_tool_content_texts("search", json!({"path": "foo.txt", "query": "dois"}));
    assert_eq!(search_texts, vec!["2 linha dois".to_string()]);

    let list_texts = mcp.call_tool_content_texts("list_files", json!({}));
    assert_eq!(list_texts, vec!["foo.txt".to_string()]);

    // Edita sem salvar, deixa o disco mudar por fora, e confere que
    // o aviso chega como um **segundo** bloco de conteúdo — nunca
    // misturado ao texto (que continua sendo o do buffer, intacto).
    mcp.call_tool_text(
        "edit",
        json!({"path": "foo.txt", "old_text": "linha um", "new_text": "linha um editada"}),
    );
    std::fs::write(&file_path, "mudou por fora\n").expect("simular mudança externa");

    let read_texts = mcp.call_tool_content_texts("read", json!({"path": "foo.txt"}));
    assert_eq!(
        read_texts.len(),
        2,
        "buffer com alterações + disco mudado: texto e aviso em blocos separados — {read_texts:?}"
    );
    assert_eq!(
        read_texts[0],
        "1 linha um editada\n2 linha dois\n3 linha tres"
    );
    assert!(
        read_texts[1].to_lowercase().contains("mudou"),
        "segundo bloco devia ser o aviso: {read_texts:?}"
    );

    mcp.kill();
}

#[test]
fn caminho_fora_da_raiz_e_recusado_via_mcp() {
    let project_dir = tempfile::tempdir().expect("diretório do projeto");
    let state_home = tempfile::tempdir().expect("diretório de estado");
    let outside = tempfile::tempdir().expect("diretório fora da raiz");
    let outside_file = outside.path().join("segredo.txt");
    std::fs::write(&outside_file, "fora da raiz\n").expect("escrever fora da raiz");

    let mut mcp = McpProcess::spawn(project_dir.path(), state_home.path(), false);
    mcp.initialize("teste-fora-da-raiz");

    let (is_error, text) =
        mcp.call_tool_text("open_file", json!({"path": outside_file.to_string_lossy()}));
    assert!(is_error, "caminho fora da raiz deveria ser recusado");
    assert!(
        text.to_lowercase().contains("raiz") || text.to_lowercase().contains("root"),
        "mensagem de recusa não menciona a raiz: {text}"
    );

    mcp.kill();
}

#[test]
fn stdout_so_contem_json_rpc() {
    let project_dir = tempfile::tempdir().expect("diretório do projeto");
    let state_home = tempfile::tempdir().expect("diretório de estado");

    let mut mcp = McpProcess::spawn(project_dir.path(), state_home.path(), false);
    // `initialize`, `tools/list` e `tools/call` (sucesso, ferramenta
    // desconhecida, argumentos inválidos) — cada `recv_line` já
    // confirma que a linha é JSON-RPC válido com `jsonrpc: "2.0"`;
    // se o panic hook ou o logging escrevessem qualquer coisa na
    // stdout, o parse falharia aqui.
    mcp.initialize("teste-stdout-limpo");
    mcp.list_tools();
    mcp.call_tool("list_files", json!({}));
    mcp.call_tool("ferramenta_que_nao_existe", json!({}));
    mcp.call_tool("open_file", json!({"caminho_errado": true}));

    mcp.kill();
}
