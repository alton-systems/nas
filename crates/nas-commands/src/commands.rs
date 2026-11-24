//! Os nove comandos iniciais (docs/decisions/0006-ia-trabalha-pelo-nas.md
//! §1/§3), cada um uma função Rust tipada registrada em
//! `register_all` — sem macro, sem núcleo em segundo plano ainda
//! (etapa 3a): chamam `Document`/`nas-history` direto, através do
//! `Workspace`.

use std::path::Path;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::context::CommandContext;
use crate::effect::Effect;
use crate::error::CommandError;
use crate::registry::{Registry, typed_command};

/// Os dez comandos: todos marcados `exposed_to_ai: true` nesta etapa
/// — a marcação já existe (ADR 0006 §5/ADR 0007 §5: "estar no
/// registro não basta"), mas nenhum comando nativo tem razão pra
/// ficar de fora ainda. Um comando de usuário (quando o Steel
/// chegar) é que nasce privado por padrão.
pub fn register_all(registry: &mut Registry) {
    registry.register(typed_command(
        "open_file",
        "Abre um arquivo do projeto (cria o buffer se ainda não estiver aberto).",
        Effect::ReadFile,
        true,
        open_file,
    ));
    registry.register(typed_command(
        "read",
        "Lê o conteúdo de um arquivo já aberto, com números de linha.",
        Effect::Pure,
        true,
        read,
    ));
    registry.register(typed_command(
        "search",
        "Busca um texto literal num arquivo já aberto, devolvendo a linha de cada ocorrência.",
        Effect::Pure,
        true,
        search,
    ));
    registry.register(typed_command(
        "list_files",
        "Lista os arquivos do projeto, respeitando .gitignore.",
        Effect::ReadFile,
        true,
        list_files,
    ));
    registry.register(typed_command(
        "edit",
        "Substitui um texto antigo exato por um novo texto num arquivo já aberto; recusa se o texto antigo não for encontrado ou aparecer mais de uma vez (use line_range pra desambiguar).",
        Effect::WriteBuffer,
        true,
        edit,
    ));
    registry.register(typed_command(
        "show_diff",
        "Mostra o diff de texto entre dois checkpoints do projeto.",
        Effect::ReadFile,
        true,
        show_diff,
    ));
    registry.register(typed_command(
        "create_file",
        "Cria um arquivo novo no projeto (recusa se já existir).",
        Effect::WriteFile,
        true,
        create_file,
    ));
    registry.register(typed_command(
        "checkpoint_create",
        "Cria um checkpoint nomeado com o conteúdo atual de todo arquivo rastreado do projeto.",
        Effect::WriteHistory,
        true,
        checkpoint_create,
    ));
    registry.register(typed_command(
        "checkpoint_restore",
        "Restaura o projeto inteiro para o estado de um checkpoint.",
        Effect::WriteFile,
        true,
        checkpoint_restore,
    ));
    registry.register(typed_command(
        "save_file",
        "Grava no disco as alterações já feitas num arquivo aberto; recusa se o arquivo mudou no disco desde a última leitura.",
        Effect::WriteFile,
        true,
        save_file,
    ));
}

/// Numera as linhas de `text` (1-indexado) — formato que as leituras
/// (`read`, cada ocorrência de `search`) devolvem, pra que a IA possa
/// montar o `line_range` de um `edit` sem contar linhas por conta
/// própria (ADR 0006 §3).
fn with_line_numbers(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let width = lines.len().max(1).to_string().len();
    lines
        .iter()
        .enumerate()
        .map(|(idx, line)| format!("{:>width$} {line}", idx + 1, width = width))
        .collect::<Vec<_>>()
        .join("\n")
}

/// O offset (em caracteres) de onde cada linha de `text` começa —
/// `offsets[0] == 0`; `offsets[n]` é o início da linha `n + 1`
/// (1-indexada). Usado por `edit` pra converter um `line_range` em
/// caracteres.
fn line_start_offsets(text: &str) -> Vec<usize> {
    let mut offsets = vec![0];
    let mut idx = 0;
    for ch in text.chars() {
        idx += 1;
        if ch == '\n' {
            offsets.push(idx);
        }
    }
    offsets
}

/// O intervalo de caracteres que cobre as linhas `from_line..=to_line`
/// (1-indexadas, inclusive) de `text`.
fn char_range_for_lines(text: &str, from_line: usize, to_line: usize) -> std::ops::Range<usize> {
    let offsets = line_start_offsets(text);
    let total = text.chars().count();
    let from_line = from_line.max(1);
    let to_line = to_line.max(from_line);
    let start = offsets.get(from_line - 1).copied().unwrap_or(total);
    let end = offsets.get(to_line).copied().unwrap_or(total);
    start..end.max(start)
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct OpenFileArgs {
    /// Caminho do arquivo, absoluto ou relativo à raiz do projeto.
    pub path: String,
}

#[derive(Debug, Serialize)]
pub struct OpenFileResult {
    pub path: String,
    /// Presente se o documento já estava aberto, o disco tinha
    /// mudado desde a última leitura, e havia alterações não salvas
    /// — foram descartadas ao recarregar (`Workspace::open_or_resync`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

fn open_file(ctx: &mut CommandContext, args: OpenFileArgs) -> Result<OpenFileResult, CommandError> {
    let path = ctx.workspace.resolve(&args.path)?;
    let warning = ctx.workspace.open_or_resync(&path)?;
    Ok(OpenFileResult {
        path: path.to_string_lossy().into_owned(),
        warning,
    })
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReadArgs {
    pub path: String,
}

#[derive(Debug, Serialize)]
pub struct ReadResult {
    /// O texto do arquivo, uma linha por linha do original, cada uma
    /// prefixada pelo seu número (1-indexado).
    pub text: String,
    /// Presente se o arquivo mudou no disco desde a última leitura
    /// e este buffer tem alterações ainda não salvas — o texto acima
    /// ainda é o do buffer, não o do disco (`Workspace::sync_with_disk`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

fn read(ctx: &mut CommandContext, args: ReadArgs) -> Result<ReadResult, CommandError> {
    let path = ctx.workspace.resolve(&args.path)?;
    let warning = ctx.workspace.sync_with_disk(&path)?;
    let document = ctx.workspace.document(&path)?;
    Ok(ReadResult {
        text: with_line_numbers(&document.full_text()),
        warning,
    })
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchArgs {
    pub path: String,
    /// Texto literal buscado — sem regex, por simplicidade.
    pub query: String,
}

#[derive(Debug, Serialize)]
pub struct SearchMatch {
    /// Número da linha (1-indexado).
    pub line: usize,
    pub text: String,
}

#[derive(Debug, Serialize)]
pub struct SearchResult {
    pub matches: Vec<SearchMatch>,
    /// Mesmo aviso de `ReadResult::warning`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

fn search(ctx: &mut CommandContext, args: SearchArgs) -> Result<SearchResult, CommandError> {
    let path = ctx.workspace.resolve(&args.path)?;
    let warning = ctx.workspace.sync_with_disk(&path)?;
    let document = ctx.workspace.document(&path)?;
    let text = document.full_text();
    let mut matches = Vec::new();
    if !args.query.is_empty() {
        for (idx, line) in text.lines().enumerate() {
            if line.contains(&args.query) {
                matches.push(SearchMatch {
                    line: idx + 1,
                    text: line.to_string(),
                });
            }
        }
    }
    Ok(SearchResult { matches, warning })
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct ListFilesArgs {
    /// Subdiretório a listar, relativo à raiz do projeto — a raiz
    /// inteira se omitido.
    #[serde(default)]
    pub dir: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ListFilesResult {
    /// Caminhos relativos à raiz do projeto, em ordem alfabética.
    pub files: Vec<String>,
}

fn list_files(
    ctx: &mut CommandContext,
    args: ListFilesArgs,
) -> Result<ListFilesResult, CommandError> {
    let root = ctx.workspace.root().to_path_buf();
    let start = match &args.dir {
        Some(dir) => ctx.workspace.resolve(dir)?,
        None => root.clone(),
    };
    let mut files = Vec::new();
    // `require_git(false)`: o nas não exige um repositório Git (ADR
    // 0005 §6 — a raiz do projeto é a do Git *se houver um*, senão o
    // diretório onde foi iniciado) — `.gitignore` deve valer também
    // sem `.git`, não só dentro de um repositório de verdade.
    for entry in ignore::WalkBuilder::new(&start).require_git(false).build() {
        let entry =
            entry.map_err(|err| CommandError::Io(std::io::Error::other(err.to_string())))?;
        if entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
            let relative = entry.path().strip_prefix(&root).unwrap_or(entry.path());
            files.push(relative.to_string_lossy().into_owned());
        }
    }
    files.sort();
    Ok(ListFilesResult { files })
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct EditArgs {
    pub path: String,
    /// O texto exato a substituir — precisa aparecer exatamente uma
    /// vez (dentro de `line_range`, se dado).
    pub old_text: String,
    pub new_text: String,
    /// Linhas `[de, até]` (1-indexadas, inclusive) pra restringir a
    /// busca, quando `old_text` aparecer mais de uma vez no arquivo
    /// inteiro.
    #[serde(default)]
    pub line_range: Option<[usize; 2]>,
}

#[derive(Debug, Serialize)]
pub struct EditResult {
    pub from: usize,
    pub to: usize,
    /// Mesmo aviso de `ReadResult::warning` — se presente, a busca
    /// por `old_text` abaixo ainda rodou contra o buffer (não contra
    /// o disco mudado), pra nunca descartar a edição pendente.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

fn edit(ctx: &mut CommandContext, args: EditArgs) -> Result<EditResult, CommandError> {
    let path = ctx.workspace.resolve(&args.path)?;
    let warning = ctx.workspace.sync_with_disk(&path)?;
    let document = ctx.workspace.document(&path)?;
    let text = document.full_text();

    let bounds = match args.line_range {
        Some([from_line, to_line]) => char_range_for_lines(&text, from_line, to_line),
        None => 0..text.chars().count(),
    };

    let haystack: Vec<char> = text.chars().collect();
    let needle: Vec<char> = args.old_text.chars().collect();
    if needle.is_empty() {
        return Err(CommandError::NoMatch);
    }

    let mut occurrences = Vec::new();
    let search_end = bounds.end.min(haystack.len());
    let mut i = bounds.start;
    while i + needle.len() <= search_end {
        if haystack[i..i + needle.len()] == needle[..] {
            occurrences.push(i);
            i += needle.len();
        } else {
            i += 1;
        }
    }

    let from = match occurrences.len() {
        0 => return Err(CommandError::NoMatch),
        1 => occurrences[0],
        count => return Err(CommandError::Ambiguous(count)),
    };
    let to = from + needle.len();

    let document = ctx.workspace.document_mut(&path).expect("checado acima");
    let change = document.replace_region(from, to, &args.new_text);
    ctx.workspace
        .record_change(ctx.author.clone(), "edit", &path, &change)?;

    Ok(EditResult { from, to, warning })
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ShowDiffArgs {
    pub from_checkpoint: String,
    pub to_checkpoint: String,
}

#[derive(Debug, Serialize)]
pub struct FileDiffSummary {
    pub path: String,
    pub change: String,
    pub unified: String,
}

#[derive(Debug, Serialize)]
pub struct ShowDiffResult {
    pub diffs: Vec<FileDiffSummary>,
}

fn show_diff(ctx: &mut CommandContext, args: ShowDiffArgs) -> Result<ShowDiffResult, CommandError> {
    let history_dir = ctx
        .workspace
        .history_dir()
        .ok_or(CommandError::HistoryUnavailable)?;
    let diffs = nas_history::diff(history_dir, &args.from_checkpoint, &args.to_checkpoint)?;
    Ok(ShowDiffResult {
        diffs: diffs
            .into_iter()
            .map(|d| FileDiffSummary {
                path: d.path,
                change: format!("{:?}", d.change),
                unified: d.unified,
            })
            .collect(),
    })
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateFileArgs {
    pub path: String,
    /// Conteúdo inicial — vazio se omitido.
    #[serde(default)]
    pub content: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CreateFileResult {
    pub path: String,
}

fn create_file(
    ctx: &mut CommandContext,
    args: CreateFileArgs,
) -> Result<CreateFileResult, CommandError> {
    let path = ctx.workspace.resolve(&args.path)?;
    ctx.workspace.create(&path)?;
    if let Some(content) = &args.content
        && !content.is_empty()
    {
        let document = ctx
            .workspace
            .document_mut(&path)
            .expect("acabou de ser criado");
        let change = document.replace_all(content);
        document.save()?;
        ctx.workspace
            .record_change(ctx.author.clone(), "create-file", &path, &change)?;
    }
    Ok(CreateFileResult {
        path: path.to_string_lossy().into_owned(),
    })
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SaveFileArgs {
    pub path: String,
}

#[derive(Debug, Serialize)]
pub struct SaveFileResult {
    pub path: String,
}

/// Grava no disco o que `edit` já mudou em memória — nunca aplica
/// nenhuma mudança de texto por conta própria. Recusa com
/// `DiskChanged` se o arquivo mudou no disco desde a última leitura
/// (`Workspace::sync_with_disk` devolvendo um aviso, não `None`) —
/// **continua recusando em qualquer tentativa seguinte**, enquanto a
/// mudança externa não for resolvida (`external_change_pending`,
/// dentro de `sync_with_disk`): salvar por cima perderia o que mudou
/// por fora — nunca sobrescreve trabalho de outro processo (revisão
/// da etapa 3c). Quem chama precisa reabrir com `open_file`
/// (`Workspace::open_or_resync`, que de fato recarrega e resolve a
/// pendência) antes de tentar salvar de novo.
fn save_file(ctx: &mut CommandContext, args: SaveFileArgs) -> Result<SaveFileResult, CommandError> {
    let path = ctx.workspace.resolve(&args.path)?;
    if ctx.workspace.sync_with_disk(&path)?.is_some() {
        return Err(CommandError::DiskChanged(path.display().to_string()));
    }
    ctx.workspace.save(&path)?;
    Ok(SaveFileResult {
        path: path.to_string_lossy().into_owned(),
    })
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CheckpointCreateArgs {
    pub name: String,
}

#[derive(Debug, Serialize)]
pub struct CheckpointCreateResult {
    pub id: String,
    pub name: String,
    pub files: usize,
}

fn checkpoint_create(
    ctx: &mut CommandContext,
    args: CheckpointCreateArgs,
) -> Result<CheckpointCreateResult, CommandError> {
    let history_dir = ctx
        .workspace
        .history_dir()
        .ok_or(CommandError::HistoryUnavailable)?
        .to_path_buf();
    let tracked = nas_history::list_tracked(&history_dir)?;
    let files: Vec<(String, Vec<u8>)> = tracked
        .iter()
        .filter_map(|entry| {
            let content = ctx.workspace.current_content_for(&entry.path)?;
            Some((entry.path.clone(), content))
        })
        .collect();
    let created = nas_history::create_checkpoint(
        &history_dir,
        &args.name,
        ctx.author.clone(),
        nas_history::now_unix(),
        false,
        &files,
    )?;
    Ok(CheckpointCreateResult {
        id: created.manifest.id,
        name: created.manifest.name,
        files: created.manifest.files.len(),
    })
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CheckpointRestoreArgs {
    pub id: String,
}

/// Como `checkpoint_restore` escreve de verdade — a mesma trait que
/// `nas-term::App` implementa pra `Ctrl+t r` (docs/decisions/0006,
/// "restaurar sem duplicação": a orquestração em si
/// (`nas_history::restore`) é uma função só, compartilhada; só isto
/// muda entre os dois lados). `current_content`/`write` reaproveitam
/// os métodos que `edit`/`create_file` já usam.
impl nas_history::RestoreTarget for CommandContext<'_> {
    fn current_content(&self, path: &str) -> Option<Vec<u8>> {
        self.workspace.current_content_for(path)
    }

    fn write(&mut self, path: &str, content: &str) -> std::io::Result<()> {
        self.workspace
            .write_file(path, content)
            .and_then(|change| {
                self.workspace.record_change(
                    self.author.clone(),
                    "restore-checkpoint",
                    Path::new(path),
                    &change,
                )
            })
            .map_err(command_error_to_io)
    }
}

fn command_error_to_io(err: CommandError) -> std::io::Error {
    match err {
        CommandError::Io(err) => err,
        other => std::io::Error::other(other.to_string()),
    }
}

fn checkpoint_restore(
    ctx: &mut CommandContext,
    args: CheckpointRestoreArgs,
) -> Result<nas_history::RestoreReport, CommandError> {
    let history_dir = ctx
        .workspace
        .history_dir()
        .ok_or(CommandError::HistoryUnavailable)?
        .to_path_buf();

    let checkpoints = nas_history::list_checkpoints(&history_dir)?;
    let checkpoint = checkpoints
        .into_iter()
        .find(|c| c.id == args.id)
        .ok_or_else(|| {
            CommandError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("checkpoint {:?} não encontrado", args.id),
            ))
        })?;

    let author = ctx.author.clone();
    let report = nas_history::restore(
        &history_dir,
        &checkpoint,
        author,
        nas_history::now_unix(),
        ctx,
    )?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use nas_history::{Author, Journal};

    use super::*;
    use crate::context::Workspace;

    /// Um projeto novo, de brinquedo, com histórico ligado — journal
    /// e `history_dir` de verdade (checkpoints precisam dele), numa
    /// pasta temporária que é limpa ao sair de escopo.
    fn workspace(project: &std::path::Path) -> Workspace {
        let history_dir = project.join(".history");
        std::fs::create_dir_all(&history_dir).expect("criar pasta de histórico");
        let journal = Journal::open(&history_dir).expect("abrir journal");
        Workspace::new(project)
            .with_history_dir(history_dir)
            .with_journal(journal)
    }

    fn ctx<'a>(workspace: &'a mut Workspace, author: Author) -> CommandContext<'a> {
        CommandContext {
            workspace,
            author,
            approved_effects: std::collections::HashSet::new(),
        }
    }

    fn write(dir: &std::path::Path, name: &str, content: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, content).expect("escrever arquivo de teste");
        path
    }

    #[test]
    fn open_file_e_read_devolvem_texto_numerado() {
        let dir = tempfile::tempdir().expect("diretório temporário");
        let path = write(dir.path(), "a.txt", "primeira\nsegunda\nterceira");
        let mut ws = workspace(dir.path());
        let mut context = ctx(&mut ws, Author::User);

        open_file(
            &mut context,
            OpenFileArgs {
                path: path.to_string_lossy().into_owned(),
            },
        )
        .expect("abrir");
        let result = read(
            &mut context,
            ReadArgs {
                path: path.to_string_lossy().into_owned(),
            },
        )
        .expect("ler");

        assert_eq!(result.text, "1 primeira\n2 segunda\n3 terceira");
    }

    #[test]
    fn read_sem_open_file_recusa() {
        let dir = tempfile::tempdir().expect("diretório temporário");
        let mut ws = workspace(dir.path());
        let mut context = ctx(&mut ws, Author::User);

        let err = read(
            &mut context,
            ReadArgs {
                path: "nunca-aberto.txt".to_string(),
            },
        )
        .expect_err("devia recusar");
        assert!(matches!(err, CommandError::NotOpen(_)));
    }

    #[test]
    fn search_devolve_a_linha_de_cada_ocorrencia() {
        let dir = tempfile::tempdir().expect("diretório temporário");
        let path = write(dir.path(), "a.txt", "um gato\num cachorro\noutro gato");
        let mut ws = workspace(dir.path());
        let mut context = ctx(&mut ws, Author::User);
        let path = path.to_string_lossy().into_owned();
        open_file(&mut context, OpenFileArgs { path: path.clone() }).expect("abrir");

        let result = search(
            &mut context,
            SearchArgs {
                path,
                query: "gato".to_string(),
            },
        )
        .expect("buscar");

        assert_eq!(result.matches.len(), 2);
        assert_eq!(result.matches[0].line, 1);
        assert_eq!(result.matches[1].line, 3);
    }

    #[test]
    fn list_files_lista_recursivo_respeitando_gitignore() {
        let dir = tempfile::tempdir().expect("diretório temporário");
        write(dir.path(), ".gitignore", "ignorado.txt\n");
        write(dir.path(), "visivel.txt", "");
        write(dir.path(), "ignorado.txt", "");
        std::fs::create_dir_all(dir.path().join("sub")).expect("criar subpasta");
        write(dir.path().join("sub").as_path(), "dentro.txt", "");
        let mut ws = workspace(dir.path());
        let mut context = ctx(&mut ws, Author::User);

        let result = list_files(&mut context, ListFilesArgs { dir: None }).expect("listar");

        assert!(result.files.contains(&"visivel.txt".to_string()));
        assert!(result.files.contains(&"sub/dentro.txt".to_string()));
        assert!(!result.files.contains(&"ignorado.txt".to_string()));
    }

    #[test]
    fn edit_substitui_texto_exato_e_journala_com_o_autor_certo() {
        let dir = tempfile::tempdir().expect("diretório temporário");
        let path = write(dir.path(), "a.txt", "ola mundo");
        let mut ws = workspace(dir.path());
        let author = Author::Ai {
            agent: "teste".to_string(),
            session: "s1".to_string(),
        };
        let mut context = ctx(&mut ws, author.clone());
        let path_str = path.to_string_lossy().into_owned();
        open_file(
            &mut context,
            OpenFileArgs {
                path: path_str.clone(),
            },
        )
        .expect("abrir");

        let result = edit(
            &mut context,
            EditArgs {
                path: path_str.clone(),
                old_text: "mundo".to_string(),
                new_text: "terra".to_string(),
                line_range: None,
            },
        )
        .expect("editar");
        assert_eq!((result.from, result.to), (4, 9));

        let text = read(
            &mut context,
            ReadArgs {
                path: path_str.clone(),
            },
        )
        .expect("ler")
        .text;
        assert_eq!(text, "1 ola terra");

        ws.journal_mut()
            .expect("journal")
            .flush(nas_history::now_unix())
            .expect("flush");
        let history_dir = ws.history_dir().expect("history_dir").to_path_buf();
        let entries = nas_history::file_history(&history_dir, &path_str).expect("ler journal");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].author, author);
        assert_eq!(entries[0].command, "edit");
    }

    #[test]
    fn edit_recusa_quando_texto_antigo_nao_existe() {
        let dir = tempfile::tempdir().expect("diretório temporário");
        let path = write(dir.path(), "a.txt", "ola mundo");
        let mut ws = workspace(dir.path());
        let mut context = ctx(&mut ws, Author::User);
        let path = path.to_string_lossy().into_owned();
        open_file(&mut context, OpenFileArgs { path: path.clone() }).expect("abrir");

        let err = edit(
            &mut context,
            EditArgs {
                path,
                old_text: "universo".to_string(),
                new_text: "terra".to_string(),
                line_range: None,
            },
        )
        .expect_err("devia recusar");
        assert!(matches!(err, CommandError::NoMatch));
    }

    #[test]
    fn edit_recusa_quando_texto_antigo_e_ambiguo_e_aceita_com_line_range() {
        let dir = tempfile::tempdir().expect("diretório temporário");
        let path = write(dir.path(), "a.txt", "gato\ncachorro\ngato");
        let mut ws = workspace(dir.path());
        let mut context = ctx(&mut ws, Author::User);
        let path = path.to_string_lossy().into_owned();
        open_file(&mut context, OpenFileArgs { path: path.clone() }).expect("abrir");

        let err = edit(
            &mut context,
            EditArgs {
                path: path.clone(),
                old_text: "gato".to_string(),
                new_text: "cão".to_string(),
                line_range: None,
            },
        )
        .expect_err("devia recusar por ambiguidade");
        assert!(matches!(err, CommandError::Ambiguous(2)));

        let result = edit(
            &mut context,
            EditArgs {
                path: path.clone(),
                old_text: "gato".to_string(),
                new_text: "cão".to_string(),
                line_range: Some([3, 3]),
            },
        )
        .expect("editar desambiguado");
        assert!(result.from > 0);

        let text = read(&mut context, ReadArgs { path }).expect("ler").text;
        assert_eq!(text, "1 gato\n2 cachorro\n3 cão");
    }

    #[test]
    fn create_file_cria_e_recusa_se_ja_existir() {
        let dir = tempfile::tempdir().expect("diretório temporário");
        let mut ws = workspace(dir.path());
        let mut context = ctx(&mut ws, Author::User);
        let path = dir.path().join("novo.txt").to_string_lossy().into_owned();

        create_file(
            &mut context,
            CreateFileArgs {
                path: path.clone(),
                content: Some("oi".to_string()),
            },
        )
        .expect("criar");
        assert_eq!(std::fs::read_to_string(&path).expect("ler"), "oi");

        let err = create_file(
            &mut context,
            CreateFileArgs {
                path,
                content: None,
            },
        )
        .expect_err("devia recusar");
        assert!(matches!(err, CommandError::Io(_)));
    }

    #[test]
    fn checkpoint_create_e_show_diff() {
        let dir = tempfile::tempdir().expect("diretório temporário");
        let path = write(dir.path(), "a.txt", "versao 1");
        let mut ws = workspace(dir.path());
        let mut context = ctx(&mut ws, Author::User);
        let path_str = path.to_string_lossy().into_owned();
        open_file(
            &mut context,
            OpenFileArgs {
                path: path_str.clone(),
            },
        )
        .expect("abrir");
        edit(
            &mut context,
            EditArgs {
                path: path_str.clone(),
                old_text: "versao 1".to_string(),
                new_text: "versao 1 editada".to_string(),
                line_range: None,
            },
        )
        .expect("editar pra rastrear o arquivo");

        let first = checkpoint_create(
            &mut context,
            CheckpointCreateArgs {
                name: "primeiro".to_string(),
            },
        )
        .expect("criar checkpoint");
        assert_eq!(first.files, 1);

        edit(
            &mut context,
            EditArgs {
                path: path_str,
                old_text: "editada".to_string(),
                new_text: "editada de novo".to_string(),
                line_range: None,
            },
        )
        .expect("editar de novo");
        let second = checkpoint_create(
            &mut context,
            CheckpointCreateArgs {
                name: "segundo".to_string(),
            },
        )
        .expect("criar segundo checkpoint");

        let diff = show_diff(
            &mut context,
            ShowDiffArgs {
                from_checkpoint: first.id,
                to_checkpoint: second.id,
            },
        )
        .expect("mostrar diff");
        assert_eq!(diff.diffs.len(), 1);
        assert_eq!(diff.diffs[0].change, "Modified");
    }

    #[test]
    fn checkpoint_restore_volta_o_arquivo_e_cria_automatico_antes_de_restaurar() {
        let dir = tempfile::tempdir().expect("diretório temporário");
        let path = write(dir.path(), "a.txt", "original");
        let mut ws = workspace(dir.path());
        let mut context = ctx(&mut ws, Author::User);
        let path_str = path.to_string_lossy().into_owned();
        open_file(
            &mut context,
            OpenFileArgs {
                path: path_str.clone(),
            },
        )
        .expect("abrir");
        edit(
            &mut context,
            EditArgs {
                path: path_str.clone(),
                old_text: "original".to_string(),
                new_text: "original rastreado".to_string(),
                line_range: None,
            },
        )
        .expect("editar pra rastrear");

        let checkpoint = checkpoint_create(
            &mut context,
            CheckpointCreateArgs {
                name: "antes-da-mudanca".to_string(),
            },
        )
        .expect("criar checkpoint");

        edit(
            &mut context,
            EditArgs {
                path: path_str.clone(),
                old_text: "rastreado".to_string(),
                new_text: "mudado".to_string(),
                line_range: None,
            },
        )
        .expect("editar de novo");

        let result = checkpoint_restore(&mut context, CheckpointRestoreArgs { id: checkpoint.id })
            .expect("restaurar");
        assert_eq!(result.restored, vec![path_str.clone()]);
        assert!(result.failed.is_empty());

        let text = read(&mut context, ReadArgs { path: path_str })
            .expect("ler")
            .text;
        assert_eq!(text, "1 original rastreado");

        let history_dir = ws.history_dir().expect("history_dir").to_path_buf();
        let checkpoints = nas_history::list_checkpoints(&history_dir).expect("listar");
        assert!(
            checkpoints
                .iter()
                .any(|c| c.automatic && c.name.starts_with("antes de restaurar"))
        );
    }

    #[test]
    fn buffer_limpo_recarrega_do_disco_antes_de_ler() {
        let dir = tempfile::tempdir().expect("diretório temporário");
        let path = write(dir.path(), "a.txt", "original");
        let mut ws = workspace(dir.path());
        let mut context = ctx(&mut ws, Author::User);
        let path_str = path.to_string_lossy().into_owned();
        open_file(
            &mut context,
            OpenFileArgs {
                path: path_str.clone(),
            },
        )
        .expect("abrir");

        std::thread::sleep(std::time::Duration::from_millis(10));
        std::fs::write(&path, "mudou por fora").expect("simular mudança externa");

        let result = read(&mut context, ReadArgs { path: path_str }).expect("ler");
        assert_eq!(result.text, "1 mudou por fora");
        assert!(result.warning.is_none());
    }

    #[test]
    fn buffer_com_alteracoes_avisa_em_vez_de_recarregar() {
        let dir = tempfile::tempdir().expect("diretório temporário");
        let path = write(dir.path(), "a.txt", "original");
        let mut ws = workspace(dir.path());
        let mut context = ctx(&mut ws, Author::User);
        let path_str = path.to_string_lossy().into_owned();
        open_file(
            &mut context,
            OpenFileArgs {
                path: path_str.clone(),
            },
        )
        .expect("abrir");
        edit(
            &mut context,
            EditArgs {
                path: path_str.clone(),
                old_text: "original".to_string(),
                new_text: "original editado".to_string(),
                line_range: None,
            },
        )
        .expect("editar");

        std::thread::sleep(std::time::Duration::from_millis(10));
        std::fs::write(&path, "mudou por fora").expect("simular mudança externa");

        let result = read(&mut context, ReadArgs { path: path_str }).expect("ler");
        assert_eq!(
            result.text, "1 original editado",
            "o buffer com alterações não deveria ter sido descartado"
        );
        assert!(result.warning.is_some());
    }

    #[test]
    fn save_file_recusado_sem_aprovacao_e_aceito_com_ela() {
        let dir = tempfile::tempdir().expect("diretório temporário");
        let path = write(dir.path(), "a.txt", "ola");
        let mut ws = workspace(dir.path());
        let author = Author::Ai {
            agent: "teste".to_string(),
            session: "s1".to_string(),
        };
        let mut context = ctx(&mut ws, author);
        let mut registry = Registry::new();
        register_all(&mut registry);

        let path_str = path.to_string_lossy().into_owned();
        registry
            .run(
                "open_file",
                &mut context,
                serde_json::json!({ "path": path_str }),
            )
            .expect("abrir");

        let err = registry
            .run(
                "save_file",
                &mut context,
                serde_json::json!({ "path": path_str }),
            )
            .expect_err("devia recusar sem aprovação");
        assert!(matches!(err, CommandError::NeedsApproval { .. }));

        context.approved_effects.insert(Effect::WriteFile);
        registry
            .run(
                "save_file",
                &mut context,
                serde_json::json!({ "path": path_str }),
            )
            .expect("devia aceitar com a aprovação concedida");
    }

    /// ADR 0006 §5 (etapa 3d): a checagem de aprovação só existe pra
    /// `Author::Ai` — uma ação do usuário, editando direto, nunca
    /// pede aprovação de si mesma, mesmo pra um efeito que pediria de
    /// um agente (`save_file`, `WriteFile`).
    #[test]
    fn acao_do_usuario_nunca_pede_aprovacao() {
        let dir = tempfile::tempdir().expect("diretório temporário");
        let path = write(dir.path(), "a.txt", "ola");
        let mut ws = workspace(dir.path());
        let mut context = ctx(&mut ws, Author::User);
        let mut registry = Registry::new();
        register_all(&mut registry);

        let path_str = path.to_string_lossy().into_owned();
        registry
            .run(
                "open_file",
                &mut context,
                serde_json::json!({ "path": path_str }),
            )
            .expect("abrir");

        registry
            .run(
                "save_file",
                &mut context,
                serde_json::json!({ "path": path_str }),
            )
            .expect("usuário nunca precisa de aprovação, mesmo sem approved_effects");
    }

    #[test]
    fn save_file_recusa_quando_o_disco_mudou_desde_a_ultima_leitura() {
        let dir = tempfile::tempdir().expect("diretório temporário");
        let path = write(dir.path(), "a.txt", "ola");
        let mut ws = workspace(dir.path());
        let mut context = ctx(&mut ws, Author::User);
        context.approved_effects.insert(Effect::WriteFile);
        let mut registry = Registry::new();
        register_all(&mut registry);

        let path_str = path.to_string_lossy().into_owned();
        registry
            .run(
                "open_file",
                &mut context,
                serde_json::json!({ "path": path_str }),
            )
            .expect("abrir");
        registry
            .run(
                "edit",
                &mut context,
                serde_json::json!({
                    "path": path_str,
                    "old_text": "ola",
                    "new_text": "ola editado",
                }),
            )
            .expect("editar");

        std::thread::sleep(std::time::Duration::from_millis(10));
        std::fs::write(&path, "mudou por fora").expect("simular mudança externa");

        let err = registry
            .run(
                "save_file",
                &mut context,
                serde_json::json!({ "path": path_str }),
            )
            .expect_err("devia recusar");
        assert!(matches!(err, CommandError::DiskChanged(_)));
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "mudou por fora",
            "nunca deveria ter sobrescrito a mudança externa"
        );

        // Tentar de novo, sem nenhuma escrita nova no meio: continua
        // recusando — a primeira tentativa não pode ter "consumido" a
        // pendência só por ter conferido o disco (o bug que motivou
        // este teste: `check_external_change` já atualiza o
        // mtime/tamanho conhecidos na primeira chamada, então uma
        // segunda checagem sem mudança nova nenhuma via `None` ali
        // mesmo com a mudança anterior ainda sem resolver).
        let err = registry
            .run(
                "save_file",
                &mut context,
                serde_json::json!({ "path": path_str }),
            )
            .expect_err("devia recusar de novo, sem nenhuma escrita nova no meio");
        assert!(matches!(err, CommandError::DiskChanged(_)));
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "mudou por fora",
            "a segunda tentativa também não pode sobrescrever a mudança externa"
        );

        // `open_file` é a saída que a mensagem de erro indica: traz o
        // conteúdo do disco, descartando a edição não salva da IA.
        let opened = registry
            .run(
                "open_file",
                &mut context,
                serde_json::json!({ "path": path_str }),
            )
            .expect("reabrir devia funcionar");
        assert!(
            opened["warning"].as_str().is_some_and(|w| !w.is_empty()),
            "reabrir devia avisar que descartou a edição não salva: {opened}"
        );
        let read_result = registry
            .run(
                "read",
                &mut context,
                serde_json::json!({ "path": path_str }),
            )
            .expect("ler devia funcionar");
        assert_eq!(read_result["text"], "1 mudou por fora");

        // Com o buffer resincronizado, editar e salvar funcionam —
        // e preservam o que o outro processo escreveu.
        registry
            .run(
                "edit",
                &mut context,
                serde_json::json!({
                    "path": path_str,
                    "old_text": "mudou por fora",
                    "new_text": "mudou por fora, e a IA editou por cima",
                }),
            )
            .expect("editar devia funcionar depois de reabrir");
        registry
            .run(
                "save_file",
                &mut context,
                serde_json::json!({ "path": path_str }),
            )
            .expect("salvar devia funcionar depois de reabrir e editar");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "mudou por fora, e a IA editou por cima",
            "o conteúdo do outro processo devia continuar na base do arquivo final"
        );
    }

    #[test]
    fn registry_expoe_os_dez_comandos_com_efeito() {
        let mut registry = Registry::new();
        register_all(&mut registry);

        assert_eq!(registry.specs().len(), 10);
        let edit_spec = registry.get("edit").expect("edit deveria existir");
        assert_eq!(edit_spec.effect, Effect::WriteBuffer);
        assert!(registry.get("comando-que-nao-existe").is_none());
        assert!(
            registry.specs().iter().all(|spec| spec.exposed_to_ai),
            "os dez comandos deveriam estar marcados como expostos à IA"
        );
    }

    #[test]
    fn registry_run_despacha_por_nome_via_json() {
        let dir = tempfile::tempdir().expect("diretório temporário");
        let path = write(dir.path(), "a.txt", "ola mundo");
        let mut ws = workspace(dir.path());
        let mut context = ctx(&mut ws, Author::User);
        let mut registry = Registry::new();
        register_all(&mut registry);

        let path_str = path.to_string_lossy().into_owned();
        registry
            .run(
                "open_file",
                &mut context,
                serde_json::json!({ "path": path_str }),
            )
            .expect("open_file via registry");
        let result = registry
            .run(
                "read",
                &mut context,
                serde_json::json!({ "path": path_str }),
            )
            .expect("read via registry");
        assert_eq!(result["text"], "1 ola mundo");

        let err = registry
            .run("inexistente", &mut context, serde_json::json!({}))
            .expect_err("devia recusar comando desconhecido");
        assert!(matches!(err, CommandError::UnknownCommand(_)));
    }

    /// A raiz de um `Workspace` de teste, numa subpasta de um
    /// `TempDir` — não o `TempDir` em si, pra ter um nível acima
    /// (fora da raiz, mas ainda dentro do `TempDir` que será limpo
    /// ao sair de escopo) onde `../` e um symlink possam apontar.
    fn project_root(dir: &std::path::Path) -> PathBuf {
        let root = dir.join("projeto");
        std::fs::create_dir_all(&root).expect("criar raiz do projeto de teste");
        root
    }

    #[test]
    fn open_file_recusa_caminho_que_sai_da_raiz_com_ponto_ponto() {
        let dir = tempfile::tempdir().expect("diretório temporário");
        let root = project_root(dir.path());
        write(dir.path(), "fora.txt", "fora da raiz");
        let mut ws = workspace(&root);
        let mut context = ctx(&mut ws, Author::User);

        let err = open_file(
            &mut context,
            OpenFileArgs {
                path: "../fora.txt".to_string(),
            },
        )
        .expect_err("devia recusar");
        assert!(matches!(err, CommandError::OutsideRoot(_)));
    }

    #[test]
    fn read_recusa_caminho_absoluto_fora_da_raiz() {
        let dir = tempfile::tempdir().expect("diretório temporário");
        let root = project_root(dir.path());
        let outside = write(dir.path(), "fora.txt", "fora da raiz");
        let mut ws = workspace(&root);
        let mut context = ctx(&mut ws, Author::User);

        let err = read(
            &mut context,
            ReadArgs {
                path: outside.to_string_lossy().into_owned(),
            },
        )
        .expect_err("devia recusar");
        assert!(matches!(err, CommandError::OutsideRoot(_)));
    }

    #[test]
    #[cfg(unix)]
    fn open_file_recusa_symlink_dentro_do_projeto_apontando_pra_fora() {
        let dir = tempfile::tempdir().expect("diretório temporário");
        let root = project_root(dir.path());
        let outside = write(dir.path(), "secreto.txt", "fora da raiz");
        std::os::unix::fs::symlink(&outside, root.join("link.txt")).expect("criar symlink");
        let mut ws = workspace(&root);
        let mut context = ctx(&mut ws, Author::User);

        let err = open_file(
            &mut context,
            OpenFileArgs {
                path: "link.txt".to_string(),
            },
        )
        .expect_err("devia recusar");
        assert!(matches!(err, CommandError::OutsideRoot(_)));
    }

    #[test]
    fn create_file_recusa_caminho_que_sai_da_raiz_com_ponto_ponto() {
        let dir = tempfile::tempdir().expect("diretório temporário");
        let root = project_root(dir.path());
        let mut ws = workspace(&root);
        let mut context = ctx(&mut ws, Author::User);

        let err = create_file(
            &mut context,
            CreateFileArgs {
                path: "../fora.txt".to_string(),
                content: None,
            },
        )
        .expect_err("devia recusar");
        assert!(matches!(err, CommandError::OutsideRoot(_)));
        assert!(!dir.path().join("fora.txt").exists());
    }

    #[test]
    fn create_file_recusa_caminho_absoluto_fora_da_raiz() {
        let dir = tempfile::tempdir().expect("diretório temporário");
        let root = project_root(dir.path());
        let outside_dir = tempfile::tempdir().expect("outro diretório temporário");
        let outside_path = outside_dir.path().join("fora.txt");
        let mut ws = workspace(&root);
        let mut context = ctx(&mut ws, Author::User);

        let err = create_file(
            &mut context,
            CreateFileArgs {
                path: outside_path.to_string_lossy().into_owned(),
                content: Some("conteudo".to_string()),
            },
        )
        .expect_err("devia recusar");
        assert!(matches!(err, CommandError::OutsideRoot(_)));
        assert!(!outside_path.exists());
    }

    #[test]
    #[cfg(unix)]
    fn create_file_recusa_symlink_pendurado_dentro_do_projeto_apontando_pra_fora() {
        let dir = tempfile::tempdir().expect("diretório temporário");
        let root = project_root(dir.path());
        let outside_dir = tempfile::tempdir().expect("outro diretório temporário");
        // o destino do link nunca é criado — testa que um link
        // *pendurado* (sem `exists() == true`) também é recusado,
        // não só um que já aponta pra um arquivo existente.
        let outside_target = outside_dir.path().join("nunca-criado.txt");
        std::os::unix::fs::symlink(&outside_target, root.join("link.txt"))
            .expect("criar symlink pendurado");
        let mut ws = workspace(&root);
        let mut context = ctx(&mut ws, Author::User);

        let err = create_file(
            &mut context,
            CreateFileArgs {
                path: "link.txt".to_string(),
                content: Some("conteudo".to_string()),
            },
        )
        .expect_err("devia recusar");
        assert!(matches!(err, CommandError::Io(_)));
        assert!(!outside_target.exists());
    }
}
