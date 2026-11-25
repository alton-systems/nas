//! Os comandos nativos (docs/decisions/0006-ia-trabalha-pelo-nas.md
//! §1/§3/§8), cada um uma função Rust tipada registrada em
//! `register_all` — sem macro, sem núcleo em segundo plano ainda
//! (etapa 3a): chamam `Document`/`nas-history`/`nas-syntax` direto,
//! através do `Workspace`.

use std::path::Path;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::context::CommandContext;
use crate::effect::Effect;
use crate::error::CommandError;
use crate::registry::{Registry, typed_command};

/// Todos os comandos nativos: marcados `exposed_to_ai: true` nesta
/// etapa — a marcação já existe (ADR 0006 §5/ADR 0007 §5: "estar no
/// registro não basta"), mas nenhum comando nativo tem razão pra
/// ficar de fora ainda. Um comando de usuário (quando o Steel
/// chegar) é que nasce privado por padrão.
pub fn register_all(registry: &mut Registry) {
    registry.register(typed_command(
        "open_file",
        "Abre um arquivo do projeto para leitura e edição.",
        Effect::ReadFile,
        true,
        open_file,
    ));
    registry.register(typed_command(
        "read",
        "Lê um arquivo com números de linha; opcionalmente só um intervalo.",
        Effect::Pure,
        true,
        read,
    ));
    registry.register(typed_command(
        "search",
        "Busca um texto literal num arquivo, devolvendo a linha de cada ocorrência.",
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
    registry.register(
        typed_command(
            "edit",
            "Substitui um texto exato por outro num arquivo; recusa se o texto não existir ou repetir. Com save, grava no disco na mesma chamada.",
            Effect::WriteBuffer,
            true,
            edit,
        )
        .with_dynamic_effect(Effect::WriteFile, |args| {
            if args
                .get("save")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false)
            {
                Effect::WriteFile
            } else {
                Effect::WriteBuffer
            }
        }),
    );
    registry.register(typed_command(
        "list_symbols",
        "Lista os símbolos de um arquivo (funções, structs, etc. em Rust; títulos em Markdown), com tipo e linhas.",
        Effect::ReadFile,
        true,
        list_symbols,
    ));
    registry.register(typed_command(
        "read_symbol",
        "Devolve o texto completo de um símbolo pelo nome; recusa listando candidatos se o nome for ambíguo.",
        Effect::ReadFile,
        true,
        read_symbol,
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
        "Cria um checkpoint nomeado com o estado atual dos arquivos rastreados.",
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
        "Grava no disco as alterações feitas num arquivo; recusa se o arquivo mudou no disco.",
        Effect::WriteFile,
        true,
        save_file,
    ));
}

/// Numera as linhas `from_line..=to_line` (1-indexadas, inclusive, já
/// dentro dos limites de `text`) de `text` com o número **absoluto**
/// de cada linha — nunca renumerado a partir de 1 quando
/// `from_line > 1` — formato que as leituras (`read`, cada ocorrência
/// de `search`, o trecho de `edit`, `read_symbol`) devolvem, pra que
/// a IA possa montar o `line_range` de um `edit` subsequente sem
/// contar linhas por conta própria (ADR 0006 §3/§8). `total_lines` só
/// entra na largura do preenchimento — pra que o número de uma linha
/// no meio do arquivo continue alinhado com o resto, mesmo quando só
/// um pedaço é devolvido.
fn numbered_lines(text: &str, from_line: usize, to_line: usize, total_lines: usize) -> String {
    let width = total_lines.max(1).to_string().len();
    text.lines()
        .enumerate()
        .skip(from_line.saturating_sub(1))
        .take(to_line.saturating_sub(from_line) + 1)
        .map(|(idx, line)| format!("{:>width$} {line}", idx + 1, width = width))
        .collect::<Vec<_>>()
        .join("\n")
}

/// A linha (1-indexada) que contém o caractere `offset` de `text`.
fn line_number_for_offset(text: &str, offset: usize) -> usize {
    let mut line = 1;
    for (count, ch) in text.chars().enumerate() {
        if count >= offset {
            break;
        }
        if ch == '\n' {
            line += 1;
        }
    }
    line
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
    // Absoluto ou relativo à raiz do projeto — todo campo `path`
    // deste registro aceita os dois (`Workspace::resolve`); descrito
    // só aqui pra não repetir a mesma frase em cada ferramenta e
    // inflar `tools/list` (ADR 0006 §8, revisão).
    #[schemars(description = "caminho do arquivo (absoluto ou relativo à raiz do projeto)")]
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

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct ReadArgs {
    #[schemars(description = "caminho do arquivo")]
    pub path: String,
    // Fora dos limites do arquivo é tolerado, nunca um erro — só
    // resulta num intervalo vazio (ADR 0006 §8, etapa 3f-prep).
    #[serde(default)]
    #[schemars(
        description = "primeira linha (1-based), opcional — lê o arquivo inteiro se omitida junto de end_line"
    )]
    pub start_line: Option<usize>,
    #[serde(default)]
    #[schemars(description = "última linha (1-based, inclusive), opcional")]
    pub end_line: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct ReadResult {
    /// O texto pedido, numerado (1-indexado, absoluto — não
    /// renumerado quando é só um intervalo). Quando `start_line`/
    /// `end_line` foi dado, a primeira linha é um cabeçalho
    /// ("linhas X–Y de N") dizendo qual intervalo saiu e quantas
    /// linhas o arquivo tem no total — omitido quando nenhum dos
    /// dois argumentos foi passado (o arquivo inteiro, como antes
    /// desta etapa).
    pub text: String,
    /// Presente se o arquivo mudou no disco desde a última leitura
    /// e este buffer tem alterações ainda não salvas — o texto acima
    /// ainda é o do buffer, não o do disco (`Workspace::sync_with_disk`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

fn read(ctx: &mut CommandContext, args: ReadArgs) -> Result<ReadResult, CommandError> {
    let path = ctx.workspace.resolve(&args.path)?;
    ctx.workspace.open(&path)?;
    let warning = ctx.workspace.sync_with_disk(&path)?;
    let document = ctx.workspace.document(&path)?;
    let full = document.full_text();
    let total = full.lines().count();

    let text = if args.start_line.is_none() && args.end_line.is_none() {
        numbered_lines(&full, 1, total.max(1), total)
    } else {
        let start = args.start_line.unwrap_or(1).max(1);
        let end = args.end_line.unwrap_or(total).min(total);
        if start > end || start > total {
            format!("linhas 0–0 de {total}\n")
        } else {
            format!(
                "linhas {start}–{end} de {total}\n{}",
                numbered_lines(&full, start, end, total)
            )
        }
    };

    Ok(ReadResult { text, warning })
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchArgs {
    pub path: String,
    // Sem regex, por simplicidade.
    #[schemars(description = "texto literal a buscar (sem regex)")]
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
    #[serde(default)]
    #[schemars(description = "subpasta a listar, opcional (padrão: a raiz do projeto)")]
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

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct EditArgs {
    pub path: String,
    // Precisa aparecer exatamente uma vez (dentro de `line_range`,
    // se dado) — `CommandError::NoMatch`/`Ambiguous` senão.
    #[schemars(description = "texto exato a substituir (deve ocorrer uma única vez)")]
    pub old_text: String,
    pub new_text: String,
    #[serde(default)]
    #[schemars(
        description = "[linha inicial, linha final] (1-based) para desambiguar quando old_text se repete"
    )]
    pub line_range: Option<[usize; 2]>,
    // Muda o efeito desta chamada de `WriteBuffer` pra `WriteFile`
    // (`CommandSpec::effective_effect`) e vale a mesma regra de
    // conflito de `save_file` (`DiskChanged` se o disco mudou) — ver
    // ADR 0006 §5/§8.
    #[serde(default)]
    #[schemars(description = "grava no disco após editar; pede permissão de escrita")]
    pub save: bool,
}

#[derive(Debug, Serialize)]
pub struct EditResult {
    pub from: usize,
    pub to: usize,
    /// `true` só quando `save` foi pedido e a gravação teve sucesso
    /// — uma recusa por conflito de disco já é um `CommandError`,
    /// nunca chega a construir este resultado.
    pub saved: bool,
    /// O trecho alterado, com até três linhas de contexto antes e
    /// depois, numerado com os mesmos números absolutos de `read` —
    /// pra a IA conferir o efeito sem precisar de um `read` extra
    /// (ADR 0006 §8, etapa 3f-prep).
    pub snippet: String,
    /// Mesmo aviso de `ReadResult::warning` — se presente, a busca
    /// por `old_text` abaixo ainda rodou contra o buffer (não contra
    /// o disco mudado), pra nunca descartar a edição pendente.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

fn edit(ctx: &mut CommandContext, args: EditArgs) -> Result<EditResult, CommandError> {
    let path = ctx.workspace.resolve(&args.path)?;
    ctx.workspace.open(&path)?;
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
    if let Some(tracking_warning) =
        ctx.workspace
            .record_change(ctx.author.clone(), "edit", &path, &change)?
    {
        ctx.warnings.push(tracking_warning);
    }

    let saved = if args.save {
        save_to_disk(ctx, &path)?;
        true
    } else {
        false
    };

    let updated_text = ctx.workspace.document(&path)?.full_text();
    let total_lines = updated_text.lines().count().max(1);
    let edit_start_line = line_number_for_offset(&updated_text, from);
    let edit_end_line = line_number_for_offset(&updated_text, from + args.new_text.chars().count());
    let context_start = edit_start_line.saturating_sub(3).max(1);
    let context_end = (edit_end_line + 3).min(total_lines);
    let snippet = numbered_lines(&updated_text, context_start, context_end, total_lines);

    Ok(EditResult {
        from,
        to,
        saved,
        snippet,
        warning,
    })
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListSymbolsArgs {
    pub path: String,
}

#[derive(Debug, Serialize)]
pub struct SymbolSummary {
    /// `"fn"`, `"method"`, `"struct"`, `"enum"`, `"trait"`, `"impl"`,
    /// `"const"` ou `"mod"` em Rust; `"heading"` em Markdown.
    pub kind: String,
    /// Já qualificado (`Tipo::metodo`) quando for um método.
    pub name: String,
    pub start_line: usize,
    pub end_line: usize,
}

impl From<nas_syntax::Symbol> for SymbolSummary {
    fn from(symbol: nas_syntax::Symbol) -> Self {
        SymbolSummary {
            kind: symbol.kind.label().to_string(),
            name: symbol.name,
            start_line: symbol.start_line,
            end_line: symbol.end_line,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ListSymbolsResult {
    pub symbols: Vec<SymbolSummary>,
}

fn list_symbols(
    ctx: &mut CommandContext,
    args: ListSymbolsArgs,
) -> Result<ListSymbolsResult, CommandError> {
    let path = ctx.workspace.resolve(&args.path)?;
    ctx.workspace.open(&path)?;
    let source = ctx.workspace.document(&path)?.full_text();
    let symbols = nas_syntax::list_symbols(&path, &source)?;
    Ok(ListSymbolsResult {
        symbols: symbols.into_iter().map(SymbolSummary::from).collect(),
    })
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReadSymbolArgs {
    pub path: String,
    // Aceita qualificado (`Tipo::metodo`) pra desambiguar entre
    // tipos diferentes com um método do mesmo nome.
    #[schemars(description = "nome do símbolo; use Tipo::metodo para desambiguar")]
    pub name: String,
}

#[derive(Debug, Serialize)]
pub struct ReadSymbolResult {
    pub kind: String,
    pub name: String,
    /// O texto do símbolo inteiro — doc comments e atributos
    /// imediatamente acima inclusos, quando houver — numerado como
    /// `read`.
    pub text: String,
}

fn read_symbol(
    ctx: &mut CommandContext,
    args: ReadSymbolArgs,
) -> Result<ReadSymbolResult, CommandError> {
    let path = ctx.workspace.resolve(&args.path)?;
    ctx.workspace.open(&path)?;
    let source = ctx.workspace.document(&path)?.full_text();
    let symbols = nas_syntax::list_symbols(&path, &source)?;

    let matches: Vec<_> = symbols
        .into_iter()
        .filter(|s| s.name == args.name)
        .collect();
    let symbol = match matches.len() {
        0 => return Err(CommandError::SymbolNotFound(args.name)),
        1 => matches.into_iter().next().expect("checado acima"),
        _ => {
            return Err(CommandError::AmbiguousSymbol(
                matches
                    .iter()
                    .map(|s| format!("{} (linha {})", s.name, s.start_line))
                    .collect(),
            ));
        }
    };

    let total = source.lines().count().max(1);
    let text = numbered_lines(&source, symbol.doc_start_line, symbol.end_line, total);
    Ok(ReadSymbolResult {
        kind: symbol.kind.label().to_string(),
        name: symbol.name,
        text,
    })
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ShowDiffArgs {
    #[schemars(description = "id do checkpoint inicial")]
    pub from_checkpoint: String,
    #[schemars(description = "id do checkpoint final")]
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
    #[serde(default)]
    #[schemars(description = "conteúdo inicial, opcional (padrão: vazio)")]
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
    if let Some(tracking_warning) = ctx.workspace.create(&path)? {
        ctx.warnings.push(tracking_warning);
    }
    if let Some(content) = &args.content
        && !content.is_empty()
    {
        let document = ctx
            .workspace
            .document_mut(&path)
            .expect("acabou de ser criado");
        let change = document.replace_all(content);
        document.save()?;
        ctx.workspace.record_last_save(&path, content, &ctx.author);
        if let Some(tracking_warning) =
            ctx.workspace
                .record_change(ctx.author.clone(), "create-file", &path, &change)?
        {
            ctx.warnings.push(tracking_warning);
        }
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
/// pendência) antes de tentar salvar de novo. Abre `path` sozinho se
/// ainda não for um buffer (ADR 0006 §8, etapa 3f-prep) — nesse caso
/// não há nada em memória além do que já está no disco, então salvar
/// é um no-op sobre o próprio conteúdo lido.
fn save_file(ctx: &mut CommandContext, args: SaveFileArgs) -> Result<SaveFileResult, CommandError> {
    let path = ctx.workspace.resolve(&args.path)?;
    ctx.workspace.open(&path)?;
    save_to_disk(ctx, &path)?;
    Ok(SaveFileResult {
        path: path.to_string_lossy().into_owned(),
    })
}

/// O corpo de `save_file`, compartilhado com `edit(save: true)` (ADR
/// 0006 §8, etapa 3f-prep) — a mesma regra de conflito de disco vale
/// pros dois. `path` já deve ser o resultado de `resolve`, e já deve
/// estar aberto.
fn save_to_disk(ctx: &mut CommandContext, path: &Path) -> Result<(), CommandError> {
    if ctx.workspace.sync_with_disk(path)?.is_some() {
        return Err(CommandError::DiskChanged(path.display().to_string()));
    }
    ctx.workspace.save(path)?;
    let content = ctx.workspace.document(path)?.full_text();
    ctx.workspace.record_last_save(path, &content, &ctx.author);
    Ok(())
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CheckpointCreateArgs {
    #[schemars(description = "nome do checkpoint")]
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
    #[schemars(description = "id do checkpoint a restaurar")]
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
        let change = self
            .workspace
            .write_file(path, content)
            .map_err(command_error_to_io)?;
        self.workspace
            .record_last_save(Path::new(path), content, &self.author);
        let tracking_warning = self
            .workspace
            .record_change(
                self.author.clone(),
                "restore-checkpoint",
                Path::new(path),
                &change,
            )
            .map_err(command_error_to_io)?;
        if let Some(tracking_warning) = tracking_warning {
            self.warnings.push(tracking_warning);
        }
        Ok(())
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
            warnings: Vec::new(),
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
                ..Default::default()
            },
        )
        .expect("ler");

        assert_eq!(result.text, "1 primeira\n2 segunda\n3 terceira");
    }

    /// ADR 0006 §8 (etapa 3f-prep): `read` abre o arquivo sozinho —
    /// não precisa mais de um `open_file` antes.
    #[test]
    fn read_abre_o_arquivo_sozinho_se_ainda_nao_estiver_aberto() {
        let dir = tempfile::tempdir().expect("diretório temporário");
        let path = write(dir.path(), "a.txt", "primeira\nsegunda");
        let mut ws = workspace(dir.path());
        let mut context = ctx(&mut ws, Author::User);

        let result = read(
            &mut context,
            ReadArgs {
                path: path.to_string_lossy().into_owned(),
                ..Default::default()
            },
        )
        .expect("ler devia abrir o arquivo sozinho");
        assert_eq!(result.text, "1 primeira\n2 segunda");
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
                save: false,
            },
        )
        .expect("editar");
        assert_eq!((result.from, result.to), (4, 9));

        let text = read(
            &mut context,
            ReadArgs {
                path: path_str.clone(),
                ..Default::default()
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
                save: false,
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
                save: false,
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
                save: false,
            },
        )
        .expect("editar desambiguado");
        assert!(result.from > 0);

        let text = read(
            &mut context,
            ReadArgs {
                path,
                ..Default::default()
            },
        )
        .expect("ler")
        .text;
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
                save: false,
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
                save: false,
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
                save: false,
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
                save: false,
            },
        )
        .expect("editar de novo");

        let result = checkpoint_restore(&mut context, CheckpointRestoreArgs { id: checkpoint.id })
            .expect("restaurar");
        assert_eq!(result.restored, vec![path_str.clone()]);
        assert!(result.failed.is_empty());

        let text = read(
            &mut context,
            ReadArgs {
                path: path_str,
                ..Default::default()
            },
        )
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

        let result = read(
            &mut context,
            ReadArgs {
                path: path_str,
                ..Default::default()
            },
        )
        .expect("ler");
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
                save: false,
            },
        )
        .expect("editar");

        std::thread::sleep(std::time::Duration::from_millis(10));
        std::fs::write(&path, "mudou por fora").expect("simular mudança externa");

        let result = read(
            &mut context,
            ReadArgs {
                path: path_str,
                ..Default::default()
            },
        )
        .expect("ler");
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
    /// quem não é `Author::User` — uma ação do usuário, editando
    /// direto, nunca pede aprovação de si mesma, mesmo pra um efeito
    /// que pediria de qualquer outro autor (`save_file`, `WriteFile`).
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

    /// ADR 0006 §5 (revisão da etapa 3d): a checagem vale pra **todo**
    /// autor que não seja `Author::User` — não só `Author::Ai`. Uma
    /// extensão (`Extension`) ou uma mudança detectada por fora
    /// (`External`) nascem bloqueadas por padrão igual a um agente,
    /// sem precisar de uma exceção própria no dia em que algo de
    /// verdade as produzir.
    #[test]
    fn extensao_e_externo_tambem_pedem_aprovacao_como_um_agente() {
        let dir = tempfile::tempdir().expect("diretório temporário");
        let path = write(dir.path(), "a.txt", "ola");
        let mut ws = workspace(dir.path());
        let mut registry = Registry::new();
        register_all(&mut registry);

        for author in [
            Author::Extension {
                name: "rustfmt".to_string(),
            },
            Author::External,
        ] {
            let mut context = ctx(&mut ws, author.clone());
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
                .expect_err(&format!("{author:?} devia precisar de aprovação"));
            assert!(matches!(err, CommandError::NeedsApproval { .. }));
        }
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
    fn read_com_intervalo_devolve_cabecalho_e_linhas_pedidas() {
        let dir = tempfile::tempdir().expect("diretório temporário");
        let path = write(dir.path(), "a.txt", "um\ndois\ntres\nquatro\ncinco");
        let mut ws = workspace(dir.path());
        let mut context = ctx(&mut ws, Author::User);
        let path_str = path.to_string_lossy().into_owned();

        let result = read(
            &mut context,
            ReadArgs {
                path: path_str,
                start_line: Some(2),
                end_line: Some(4),
            },
        )
        .expect("ler intervalo");

        assert_eq!(result.text, "linhas 2–4 de 5\n2 dois\n3 tres\n4 quatro");
    }

    #[test]
    fn read_com_intervalo_fora_dos_limites_e_tolerante() {
        let dir = tempfile::tempdir().expect("diretório temporário");
        let path = write(dir.path(), "a.txt", "um\ndois\ntres");
        let mut ws = workspace(dir.path());
        let mut context = ctx(&mut ws, Author::User);
        let path_str = path.to_string_lossy().into_owned();

        // end_line além do fim do arquivo: clampa pro total, sem erro.
        let result = read(
            &mut context,
            ReadArgs {
                path: path_str.clone(),
                start_line: Some(2),
                end_line: Some(100),
            },
        )
        .expect("ler intervalo além do fim");
        assert_eq!(result.text, "linhas 2–3 de 3\n2 dois\n3 tres");

        // start_line além do fim do arquivo: intervalo vazio, sem erro.
        let result = read(
            &mut context,
            ReadArgs {
                path: path_str,
                start_line: Some(50),
                end_line: None,
            },
        )
        .expect("ler com start_line fora dos limites");
        assert_eq!(result.text, "linhas 0–0 de 3\n");
    }

    /// ADR 0006 §8 (etapa 3f-prep): `edit` abre o arquivo sozinho —
    /// não precisa mais de um `open_file` antes.
    #[test]
    fn edit_abre_o_arquivo_sozinho_se_ainda_nao_estiver_aberto() {
        let dir = tempfile::tempdir().expect("diretório temporário");
        let path = write(dir.path(), "a.txt", "ola mundo");
        let mut ws = workspace(dir.path());
        let mut context = ctx(&mut ws, Author::User);

        let result = edit(
            &mut context,
            EditArgs {
                path: path.to_string_lossy().into_owned(),
                old_text: "mundo".to_string(),
                new_text: "terra".to_string(),
                line_range: None,
                save: false,
            },
        )
        .expect("editar devia abrir o arquivo sozinho");
        assert_eq!((result.from, result.to), (4, 9));
        assert!(!result.saved);
    }

    /// ADR 0006 §8 (etapa 3f-prep): `save_file` abre o arquivo
    /// sozinho — como não há nada em memória além do que já está no
    /// disco, salvar é um no-op sobre o próprio conteúdo lido.
    #[test]
    fn save_file_abre_o_arquivo_sozinho_se_ainda_nao_estiver_aberto() {
        let dir = tempfile::tempdir().expect("diretório temporário");
        let path = write(dir.path(), "a.txt", "ola");
        let mut ws = workspace(dir.path());
        let mut context = ctx(&mut ws, Author::User);

        save_file(
            &mut context,
            SaveFileArgs {
                path: path.to_string_lossy().into_owned(),
            },
        )
        .expect("salvar devia abrir o arquivo sozinho");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "ola");
    }

    /// ADR 0006 §8 (etapa 3f-prep): o resultado de `edit` inclui o
    /// trecho alterado com até três linhas de contexto antes e
    /// depois, numeradas com os números absolutos do arquivo.
    #[test]
    fn edit_devolve_trecho_com_contexto() {
        let dir = tempfile::tempdir().expect("diretório temporário");
        let text = "linha1\nlinha2\nlinha3\nALVO\nlinha5\nlinha6\nlinha7\nlinha8";
        let path = write(dir.path(), "a.txt", text);
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

        let result = edit(
            &mut context,
            EditArgs {
                path: path_str,
                old_text: "ALVO".to_string(),
                new_text: "MUDOU".to_string(),
                line_range: None,
                save: false,
            },
        )
        .expect("editar");

        assert_eq!(
            result.snippet,
            "1 linha1\n2 linha2\n3 linha3\n4 MUDOU\n5 linha5\n6 linha6\n7 linha7"
        );
    }

    /// ADR 0006 §5/§8 (etapa 3f-prep): `edit` com `save: true`, pedido
    /// por um autor que não é o usuário, pede aprovação de
    /// `write-file` como `save_file` já pedia — e, sem aprovação, nem
    /// chega a gravar no disco (a edição em memória continuaria
    /// isolada caso a chamada tivesse sido aceita só parcialmente, o
    /// que nunca acontece aqui: o `Registry` recusa antes de chamar
    /// `edit` de verdade).
    #[test]
    fn edit_com_save_pede_aprovacao_de_write_file_pra_quem_nao_e_usuario() {
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

        // Sem save: edita livremente, sem aprovação nenhuma — o
        // efeito continua WriteBuffer.
        registry
            .run(
                "edit",
                &mut context,
                serde_json::json!({ "path": path_str, "old_text": "ola", "new_text": "oi" }),
            )
            .expect("editar sem save não devia precisar de aprovação");

        // Com save: true, pede aprovação de write-file.
        let err = registry
            .run(
                "edit",
                &mut context,
                serde_json::json!({ "path": path_str, "old_text": "oi", "new_text": "oi de novo", "save": true }),
            )
            .expect_err("devia recusar sem aprovação de write-file");
        assert!(matches!(
            err,
            CommandError::NeedsApproval {
                effect: Effect::WriteFile,
                ..
            }
        ));
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "ola",
            "nada devia ter ido pro disco"
        );

        // Com a aprovação concedida, grava no disco na mesma chamada.
        context.approved_effects.insert(Effect::WriteFile);
        let result = registry
            .run(
                "edit",
                &mut context,
                serde_json::json!({ "path": path_str, "old_text": "oi", "new_text": "oi de novo", "save": true }),
            )
            .expect("devia aceitar com a aprovação concedida");
        assert_eq!(result["saved"], true);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "oi de novo",
            "devia ter gravado no disco nesta mesma chamada"
        );
    }

    /// ADR 0006 §8 (etapa 3f-prep): a mesma regra de conflito de
    /// disco de `save_file` vale pro `save` de `edit` — se o buffer
    /// já tinha uma edição não salva quando o disco mudou por fora
    /// (o mesmo cenário que `save_file` recusa), a edição em memória
    /// desta chamada ainda acontece, mas a chamada inteira recusa
    /// (nada vai pro disco).
    #[test]
    fn edit_com_save_recusa_se_o_disco_mudou() {
        let dir = tempfile::tempdir().expect("diretório temporário");
        let path = write(dir.path(), "a.txt", "ola");
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
                old_text: "ola".to_string(),
                new_text: "ola editado".to_string(),
                line_range: None,
                save: false,
            },
        )
        .expect("primeira edição, só em memória");

        std::thread::sleep(std::time::Duration::from_millis(10));
        std::fs::write(&path, "mudou por fora").expect("simular mudança externa");

        let err = edit(
            &mut context,
            EditArgs {
                path: path_str,
                old_text: "editado".to_string(),
                new_text: "editado e a IA editou de novo".to_string(),
                line_range: None,
                save: true,
            },
        )
        .expect_err("devia recusar o save por conflito de disco");
        assert!(matches!(err, CommandError::DiskChanged(_)));
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "mudou por fora",
            "nunca deveria ter sobrescrito a mudança externa"
        );
    }

    /// ADR 0006 §8 (etapa 3f-prep): `list_symbols`/`read_symbol` num
    /// arquivo Rust real do próprio repositório (este mesmo crate,
    /// `effect.rs`, copiado pra dentro da raiz do `Workspace` de
    /// teste — `resolve` recusaria o caminho original, fora dela) —
    /// método qualificado e doc comments inclusos.
    #[test]
    fn list_symbols_e_read_symbol_em_arquivo_rust_real_do_repositorio() {
        let real_content = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/effect.rs"),
        )
        .expect("ler o arquivo real do repositório");
        let dir = tempfile::tempdir().expect("diretório temporário");
        let path = write(dir.path(), "effect.rs", &real_content);
        let mut ws = workspace(dir.path());
        let mut context = ctx(&mut ws, Author::User);
        let path_str = path.to_string_lossy().into_owned();

        let listed = list_symbols(
            &mut context,
            ListSymbolsArgs {
                path: path_str.clone(),
            },
        )
        .expect("listar símbolos");
        let effect_enum = listed
            .symbols
            .iter()
            .find(|s| s.name == "Effect")
            .expect("deveria achar o enum Effect");
        assert_eq!(effect_enum.kind, "enum");
        assert!(
            listed
                .symbols
                .iter()
                .any(|s| s.name == "Effect::policy_key" && s.kind == "method"),
            "deveria achar o método qualificado Effect::policy_key: {listed:?}"
        );

        let read = read_symbol(
            &mut context,
            ReadSymbolArgs {
                path: path_str,
                name: "Effect::policy_key".to_string(),
            },
        )
        .expect("ler o símbolo qualificado");
        assert_eq!(read.kind, "method");
        assert!(
            read.text.contains("pedem aprovação"),
            "devia incluir o doc comment logo acima: {}",
            read.text
        );
    }

    #[test]
    fn read_symbol_recusa_nome_ambiguo_listando_candidatos() {
        let dir = tempfile::tempdir().expect("diretório temporário");
        let path = write(
            dir.path(),
            "a.rs",
            "pub struct Dup { x: i32 }\n\npub struct Dup { y: i32 }\n",
        );
        let mut ws = workspace(dir.path());
        let mut context = ctx(&mut ws, Author::User);
        let path_str = path.to_string_lossy().into_owned();

        let err = read_symbol(
            &mut context,
            ReadSymbolArgs {
                path: path_str,
                name: "Dup".to_string(),
            },
        )
        .expect_err("devia recusar por ambiguidade");
        match err {
            CommandError::AmbiguousSymbol(candidates) => {
                assert_eq!(candidates.len(), 2);
                assert!(candidates[0].contains("linha 1"));
                assert!(candidates[1].contains("linha 3"));
            }
            other => panic!("esperava AmbiguousSymbol, veio {other:?}"),
        }
    }

    #[test]
    fn read_symbol_recusa_nome_inexistente() {
        let dir = tempfile::tempdir().expect("diretório temporário");
        let path = write(dir.path(), "a.rs", "pub fn existe() {}\n");
        let mut ws = workspace(dir.path());
        let mut context = ctx(&mut ws, Author::User);
        let path_str = path.to_string_lossy().into_owned();

        let err = read_symbol(
            &mut context,
            ReadSymbolArgs {
                path: path_str,
                name: "nao_existe".to_string(),
            },
        )
        .expect_err("devia recusar");
        assert!(matches!(err, CommandError::SymbolNotFound(_)));
    }

    #[test]
    fn list_symbols_em_markdown_usa_titulos_como_simbolos() {
        let dir = tempfile::tempdir().expect("diretório temporário");
        let path = write(
            dir.path(),
            "a.md",
            "# Título\ntexto\n## Subtítulo\nmais texto\n",
        );
        let mut ws = workspace(dir.path());
        let mut context = ctx(&mut ws, Author::User);
        let path_str = path.to_string_lossy().into_owned();

        let result = list_symbols(&mut context, ListSymbolsArgs { path: path_str })
            .expect("listar símbolos do Markdown");

        assert_eq!(result.symbols.len(), 2);
        assert_eq!(result.symbols[0].kind, "heading");
        assert_eq!(result.symbols[0].name, "Título");
        assert_eq!(result.symbols[1].name, "Subtítulo");
    }

    #[test]
    fn list_symbols_recusa_linguagem_nao_suportada() {
        let dir = tempfile::tempdir().expect("diretório temporário");
        let path = write(dir.path(), "a.py", "def f():\n    pass\n");
        let mut ws = workspace(dir.path());
        let mut context = ctx(&mut ws, Author::User);
        let path_str = path.to_string_lossy().into_owned();

        let err = list_symbols(&mut context, ListSymbolsArgs { path: path_str })
            .expect_err("devia recusar linguagem não suportada");
        assert!(matches!(err, CommandError::UnsupportedLanguage(_)));
    }

    /// ADR 0006 §6 (etapa 3e): a primeira mudança efetiva de uma
    /// sessão de IA cria um checkpoint automático "antes da sessão de
    /// <agent>" — uma segunda mudança efetiva, na mesma sessão, não
    /// cria um segundo.
    #[test]
    fn primeira_mudanca_efetiva_de_uma_sessao_cria_checkpoint_automatico_uma_vez_so() {
        let dir = tempfile::tempdir().expect("diretório temporário");
        let path = write(dir.path(), "a.txt", "ola");
        let mut ws = workspace(dir.path());
        let author = Author::Ai {
            agent: "claude-code".to_string(),
            session: "s1".to_string(),
        };
        let mut context = ctx(&mut ws, author.clone());
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
                serde_json::json!({ "path": path_str, "old_text": "ola", "new_text": "oi" }),
            )
            .expect("editar");
        registry
            .run(
                "edit",
                &mut context,
                serde_json::json!({ "path": path_str, "old_text": "oi", "new_text": "oi de novo" }),
            )
            .expect("editar de novo, mesma sessão");

        let history_dir = ws.history_dir().expect("history_dir").to_path_buf();
        let checkpoints = nas_history::list_checkpoints(&history_dir).expect("listar checkpoints");
        let session_checkpoints: Vec<_> = checkpoints
            .iter()
            .filter(|c| c.automatic && c.author == author)
            .collect();
        assert_eq!(
            session_checkpoints.len(),
            1,
            "deveria existir exatamente um checkpoint de sessão, mesmo com duas edições: {checkpoints:?}"
        );
        assert!(
            session_checkpoints[0]
                .name
                .starts_with("antes da sessão de claude-code"),
            "{}",
            session_checkpoints[0].name
        );
    }

    /// ADR 0006 §6 (etapa 3e): o autor é `Author::User`, nunca ganha
    /// o checkpoint automático de sessão — só sessão de IA tem um.
    #[test]
    fn usuario_editando_direto_nao_cria_checkpoint_de_sessao() {
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
                "edit",
                &mut context,
                serde_json::json!({ "path": path_str, "old_text": "ola", "new_text": "oi" }),
            )
            .expect("editar");

        let history_dir = ws.history_dir().expect("history_dir").to_path_buf();
        let checkpoints = nas_history::list_checkpoints(&history_dir).expect("listar checkpoints");
        assert!(
            checkpoints.is_empty(),
            "usuário editando direto não deveria criar checkpoint algum: {checkpoints:?}"
        );
    }

    /// ADR 0006 §6: restaurar o checkpoint de sessão desfaz tudo que
    /// a sessão mudou — reverte o que já existia (`a.txt`) e manda
    /// pra lixeira o que a sessão criou (`b.txt`).
    #[test]
    fn restaurar_checkpoint_de_sessao_desfaz_tudo_que_ela_mudou() {
        let dir = tempfile::tempdir().expect("diretório temporário");
        let path_a = write(dir.path(), "a.txt", "a original");
        let mut ws = workspace(dir.path());
        let history_dir = ws.history_dir().expect("history_dir").to_path_buf();
        let author = Author::Ai {
            agent: "claude-code".to_string(),
            session: "s1".to_string(),
        };
        let mut context = ctx(&mut ws, author.clone());
        context.approved_effects.insert(Effect::WriteFile);
        let mut registry = Registry::new();
        register_all(&mut registry);
        let path_a_str = path_a.to_string_lossy().into_owned();

        registry
            .run(
                "open_file",
                &mut context,
                serde_json::json!({ "path": path_a_str }),
            )
            .expect("abrir");
        registry
            .run(
                "edit",
                &mut context,
                serde_json::json!({ "path": path_a_str, "old_text": "a original", "new_text": "a editado" }),
            )
            .expect("editar");
        registry
            .run(
                "save_file",
                &mut context,
                serde_json::json!({ "path": path_a_str }),
            )
            .expect("salvar");

        let path_b = dir.path().join("b.txt");
        let path_b_str = path_b.to_string_lossy().into_owned();
        registry
            .run(
                "create_file",
                &mut context,
                serde_json::json!({ "path": path_b_str, "content": "arquivo novo da sessão" }),
            )
            .expect("criar");

        let checkpoints = nas_history::list_checkpoints(&history_dir).expect("listar checkpoints");
        let session_checkpoint = checkpoints
            .iter()
            .find(|c| c.automatic && c.author == author)
            .expect("deveria existir um checkpoint de sessão")
            .clone();

        // Isola a lixeira de verdade do usuário rodando o teste — a
        // restauração abaixo vai mover "b.txt" pra ela (criado só
        // depois do checkpoint de sessão).
        let trash_home = tempfile::tempdir().expect("diretório temporário");
        unsafe {
            std::env::set_var("XDG_DATA_HOME", trash_home.path());
        }

        let report = nas_history::restore(
            &history_dir,
            &session_checkpoint,
            Author::User,
            nas_history::now_unix(),
            &mut context,
        )
        .expect("restaurar");

        assert_eq!(report.restored, vec![path_a_str.clone()]);
        assert_eq!(report.trashed, vec![path_b_str]);
        assert_eq!(std::fs::read_to_string(&path_a).unwrap(), "a original");
        assert!(
            !path_b.exists(),
            "b.txt deveria ter sido movido pra lixeira"
        );
    }

    /// Issue #24: `ensure_tracked` falhando (aqui, forçado: um
    /// arquivo comum no lugar de `originals/`) não trava o comando
    /// nem é descartado em silêncio — o resultado continua de
    /// sucesso, com o aviso no campo `warning`.
    #[test]
    fn ensure_tracked_falhando_aparece_como_aviso_no_resultado() {
        let dir = tempfile::tempdir().expect("diretório temporário");
        let path = write(dir.path(), "a.txt", "ola");
        let mut ws = workspace(dir.path());
        let history_dir = ws.history_dir().expect("history_dir").to_path_buf();
        std::fs::write(history_dir.join("originals"), "não é uma pasta").expect("forçar a colisão");

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
        let result = registry
            .run(
                "edit",
                &mut context,
                serde_json::json!({ "path": path_str, "old_text": "ola", "new_text": "oi" }),
            )
            .expect("editar deveria funcionar mesmo com o aviso");

        assert!(
            result["warning"]
                .as_str()
                .is_some_and(|w| w.to_lowercase().contains("histórico")),
            "deveria avisar que não conseguiu guardar o original: {result}"
        );
    }

    #[test]
    fn registry_expoe_os_doze_comandos_com_efeito() {
        let mut registry = Registry::new();
        register_all(&mut registry);

        assert_eq!(registry.specs().len(), 12);
        let edit_spec = registry.get("edit").expect("edit deveria existir");
        assert_eq!(edit_spec.effect, Effect::WriteBuffer);
        assert!(registry.get("comando-que-nao-existe").is_none());
        assert!(
            registry.specs().iter().all(|spec| spec.exposed_to_ai),
            "os doze comandos deveriam estar marcados como expostos à IA"
        );
    }

    /// ADR 0006 §5/§8 (etapa 3f-prep): o efeito de `edit` depende do
    /// argumento `save` — `max_effect` (usado só pra anotação MCP
    /// estática) é sempre o pior caso (`WriteFile`), mas
    /// `effective_effect` por chamada muda com o argumento.
    #[test]
    fn edit_tem_efeito_dependente_do_argumento_save() {
        let mut registry = Registry::new();
        register_all(&mut registry);
        let edit_spec = registry.get("edit").expect("edit deveria existir");

        assert_eq!(edit_spec.max_effect, Effect::WriteFile);
        assert_eq!(
            edit_spec.effective_effect(&serde_json::json!({})),
            Effect::WriteBuffer
        );
        assert_eq!(
            edit_spec.effective_effect(&serde_json::json!({ "save": false })),
            Effect::WriteBuffer
        );
        assert_eq!(
            edit_spec.effective_effect(&serde_json::json!({ "save": true })),
            Effect::WriteFile
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
                ..Default::default()
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
