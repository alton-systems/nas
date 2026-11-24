use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};

use nas_editor::{Document, TextChange};
use nas_history::{Author, Journal};

use crate::error::CommandError;

/// O conjunto de documentos abertos de um projeto, mais onde fica o
/// histórico dele — o que os comandos (`crate::commands`) operam
/// sobre. Hoje só em processo (etapa 3a); é o que o núcleo em
/// segundo plano (ADR 0006 §2) vai expor por um socket numa etapa
/// futura (3b), sem mudar a API dos comandos em si.
pub struct Workspace {
    root: PathBuf,
    history_dir: Option<PathBuf>,
    journal: Option<Journal>,
    documents: HashMap<PathBuf, Document>,
}

impl Workspace {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Workspace {
            root: root.into(),
            history_dir: None,
            journal: None,
            documents: HashMap::new(),
        }
    }

    pub fn with_history_dir(mut self, history_dir: impl Into<PathBuf>) -> Self {
        self.history_dir = Some(history_dir.into());
        self
    }

    pub fn with_journal(mut self, journal: Journal) -> Self {
        self.journal = Some(journal);
        self
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn history_dir(&self) -> Option<&Path> {
        self.history_dir.as_deref()
    }

    pub fn journal_mut(&mut self) -> Option<&mut Journal> {
        self.journal.as_mut()
    }

    /// Resolve `path` (absoluto, ou relativo à raiz do projeto) e
    /// confere que o resultado continua **dentro** da raiz — segue
    /// `..` e links simbólicos (`canonicalize`, tolerando que o
    /// próprio caminho ainda não exista, pra `create_file`) antes de
    /// comparar, pra que nem um link dentro do projeto apontando pra
    /// fora, nem um `../fora-da-raiz`, escapem. Todo caminho que um
    /// comando recebe passa por aqui antes de tocar o disco — nunca
    /// usado como string crua.
    pub fn resolve(&self, path: &str) -> Result<PathBuf, CommandError> {
        let candidate = Path::new(path);
        let joined = if candidate.is_absolute() {
            candidate.to_path_buf()
        } else {
            self.root.join(candidate)
        };

        let canonical_root = self.root.canonicalize()?;
        let canonical = canonicalize_lenient(&joined)?;

        if !canonical.starts_with(&canonical_root) {
            return Err(CommandError::OutsideRoot(path.to_string()));
        }
        Ok(canonical)
    }

    /// Registra uma mudança de texto no journal (ADR 0005 §1/§2) —
    /// sem efeito se não houver journal configurado (ex.: num
    /// `Workspace` de teste que não precisa de histórico), ou se a
    /// mudança for vazia (nada a registrar). Também marca `path`
    /// como rastreado na primeira vez (`nas_history::ensure_tracked`),
    /// igual a `App::journal_record_for` em `nas-term`. `path` já
    /// deve ser o resultado de `resolve` — este método não valida de
    /// novo contra a raiz.
    pub fn record_change(
        &mut self,
        author: Author,
        command: &str,
        path: &Path,
        change: &TextChange,
    ) -> Result<(), CommandError> {
        if change.from == change.to && change.removed.is_empty() && change.inserted.is_empty() {
            return Ok(());
        }
        let file = path.to_string_lossy().into_owned();
        if let Some(history_dir) = &self.history_dir {
            let disk_content = std::fs::read(path).ok();
            let _ = nas_history::ensure_tracked(history_dir, &file, disk_content.as_deref());
        }
        if let Some(journal) = &mut self.journal {
            journal.record(
                nas_history::now_unix(),
                author,
                command.to_string(),
                file,
                change.from,
                change.to,
                change.removed.clone(),
                change.inserted.clone(),
            )?;
        }
        Ok(())
    }

    /// `path` já deve ser o resultado de `resolve`.
    pub fn document(&self, path: &Path) -> Result<&Document, CommandError> {
        self.documents
            .get(path)
            .ok_or_else(|| CommandError::NotOpen(path.display().to_string()))
    }

    /// `path` já deve ser o resultado de `resolve`.
    pub fn document_mut(&mut self, path: &Path) -> Result<&mut Document, CommandError> {
        self.documents
            .get_mut(path)
            .ok_or_else(|| CommandError::NotOpen(path.display().to_string()))
    }

    /// Abre `path` como documento, se ainda não estiver — idempotente:
    /// chamar de novo com o mesmo caminho não reabre (perderia
    /// alterações em memória não salvas). `path` já deve ser o
    /// resultado de `resolve`.
    pub fn open(&mut self, path: &Path) -> Result<&Document, CommandError> {
        if !self.documents.contains_key(path) {
            let document = Document::open(path)?;
            self.documents.insert(path.to_path_buf(), document);
        }
        Ok(self.documents.get(path).expect("acabou de ser inserido"))
    }

    /// Como `open`, mas pra quem pede explicitamente pra (re)abrir um
    /// arquivo (`open_file`) — diferente de `sync_with_disk`, usado
    /// implicitamente por `read`/`search`/`edit`, que nunca descarta
    /// uma alteração não salva da IA sem avisar e esperar uma decisão.
    /// Pedir pra abrir de novo **é** a decisão: se o documento já
    /// estava aberto e o disco mudou desde a última leitura, recarrega
    /// — mesmo que haja alterações não salvas, que são descartadas —
    /// e devolve um aviso dizendo isso, pra quem chamou saber que
    /// precisa refazer a edição sobre o conteúdo atual (`save_file`
    /// aponta exatamente aqui quando recusa por causa de uma mudança
    /// externa, então esta é a rota que de fato resolve aquele
    /// impasse). `path` já deve ser o resultado de `resolve`.
    pub fn open_or_resync(&mut self, path: &Path) -> Result<Option<String>, CommandError> {
        let Some(document) = self.documents.get_mut(path) else {
            self.open(path)?;
            return Ok(None);
        };
        document.check_external_change()?;
        if !document.external_change_pending() {
            return Ok(None);
        }
        let had_unsaved_changes = document.is_modified();
        document.reload()?;
        if had_unsaved_changes {
            return Ok(Some(format!(
                "{} mudou no disco — as alterações ainda não salvas deste buffer foram descartadas; releia antes de editar de novo",
                path.display()
            )));
        }
        Ok(None)
    }

    /// Cria um documento novo, vazio, em `path` — pra `create_file`.
    /// Diferente de `open`: recusa se `path` já existir no disco ou
    /// já estiver aberto, pra `create_file` nunca sobrescrever algo
    /// sem querer (quem quer sobrescrever usa `edit`/`open_file`).
    /// `path` já deve ser o resultado de `resolve`.
    pub fn create(&mut self, path: &Path) -> Result<(), CommandError> {
        // `symlink_metadata` (não `exists`, que segue o link): um
        // link pendurado (apontando pra um destino que ainda não
        // existe, inclusive fora da raiz) não aparece em `exists()`,
        // mas escrever ali seguiria o link mesmo assim — recusar
        // qualquer coisa já presente, pendurada ou não, fecha essa
        // brecha sem precisar resolver o destino do link à mão.
        if path.symlink_metadata().is_ok() || self.documents.contains_key(path) {
            return Err(CommandError::Io(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("{} já existe", path.display()),
            )));
        }
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, "")?;
        let document = Document::open(path)?;
        self.documents.insert(path.to_path_buf(), document);
        Ok(())
    }

    /// O conteúdo atual de um caminho rastreado — do documento aberto,
    /// se houver (pode ter alterações em memória ainda não salvas),
    /// senão do disco. Usado por `checkpoint_create`/a restauração,
    /// que precisam do estado de **todo** arquivo rastreado (ADR 0005
    /// §4), não só dos documentos abertos. Aceita o caminho como
    /// `nas-history` guarda (uma `String`, já canônica desde que foi
    /// rastreado por `record_change`) — não passa por `resolve` de
    /// novo.
    pub fn current_content_for(&self, path: &str) -> Option<Vec<u8>> {
        if let Some(document) = self.documents.get(Path::new(path)) {
            return Some(document.full_text().into_bytes());
        }
        std::fs::read(path).ok()
    }

    /// Antes de qualquer comando que opera sobre um documento já
    /// aberto (`read`, `search`, `edit`, `save_file`): confere se o
    /// arquivo mudou no disco desde a última leitura/escrita, via a
    /// mesma detecção de mudança externa que `nas-term` já usa
    /// (`nas_editor::Document::check_external_change`, ADR 0005 §1).
    ///
    /// Buffer sem alterações deste processo: nada a perder —
    /// recarrega sozinho, devolve `None` (nada a avisar). Buffer com
    /// alterações ainda não salvas: não descarta nada — devolve uma
    /// mensagem pra o comando incluir no resultado (`save_file` trata
    /// isto como erro, os demais só avisam).
    pub fn sync_with_disk(&mut self, path: &Path) -> Result<Option<String>, CommandError> {
        let Some(document) = self.documents.get_mut(path) else {
            return Ok(None);
        };
        document.check_external_change()?;
        // `external_change_pending`, não o retorno de `check_external_change`
        // acima: a primeira chamada já atualiza o mtime/tamanho conhecidos
        // (pra não reportar a mesma mudança duas vezes como "nova"), então
        // uma segunda chamada sem nenhuma escrita nova no meio veria `None`
        // ali mesmo com a mudança anterior ainda sem resolver — é essa
        // bandeira, não a detecção pontual, que diz se ainda há uma mudança
        // pendente.
        if !document.external_change_pending() {
            return Ok(None);
        }
        if document.is_modified() {
            return Ok(Some(format!(
                "{} mudou no disco desde a última leitura — este buffer tem alterações ainda não salvas, nada foi recarregado",
                path.display()
            )));
        }
        document.reload()?;
        Ok(None)
    }

    /// Escreve `content` em `path`: se já há um documento aberto pra
    /// esse caminho, substitui o conteúdo dele (`replace_all`, que
    /// entra no desfazer por tecla) e salva; senão, abre um documento
    /// descartável só pra escrever — restaurar/recriar afeta o
    /// arquivo, não só um documento que porventura esteja aberto.
    /// Mesma observação de `current_content_for` sobre `path` já
    /// canônico.
    pub fn write_file(&mut self, path: &str, content: &str) -> Result<TextChange, CommandError> {
        if let Some(document) = self.documents.get_mut(Path::new(path)) {
            let change = document.replace_all(content);
            document.save()?;
            return Ok(change);
        }
        let mut document = Document::open(path)?;
        let change = document.replace_all(content);
        document.save()?;
        self.documents.insert(PathBuf::from(path), document);
        Ok(change)
    }

    /// Grava no disco o conteúdo que já está no documento aberto em
    /// `path` (as edições que `edit` já fez em memória) — pra
    /// `save_file`. Diferente de `write_file`: não substitui o
    /// conteúdo por nada antes, só persiste o que já está lá. `path`
    /// já deve ser o resultado de `resolve`, e já deve estar aberto
    /// (`document_mut` erra com `NotOpen` senão).
    pub fn save(&mut self, path: &Path) -> Result<(), CommandError> {
        self.document_mut(path)?.save()?;
        Ok(())
    }
}

/// Como `Path::canonicalize`, mas tolera que o próprio caminho (ou
/// algum sufixo dele) ainda não exista — pra `create_file`, ou pra um
/// caminho restaurado em subpastas novas. Canonicaliza o ancestral
/// mais próximo que existir (resolvendo `..` e links simbólicos nele)
/// e reanexa o resto por cima, sem tentar resolver um componente que
/// ainda não existe.
fn canonicalize_lenient(path: &Path) -> io::Result<PathBuf> {
    if let Ok(canonical) = path.canonicalize() {
        return Ok(canonical);
    }
    let mut remainder = Vec::new();
    let mut ancestor = path.to_path_buf();
    loop {
        let file_name = ancestor
            .file_name()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "caminho inválido"))?
            .to_owned();
        remainder.push(file_name);
        let Some(parent) = ancestor.parent().map(Path::to_path_buf) else {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "nenhum ancestral existente",
            ));
        };
        if let Ok(canonical_parent) = parent.canonicalize() {
            let mut result = canonical_parent;
            for part in remainder.into_iter().rev() {
                result.push(part);
            }
            return Ok(result);
        }
        ancestor = parent;
    }
}

/// O que um comando recebe pra agir: o `Workspace` (§ acima) mais
/// quem está chamando — todo comando que muda texto/disco journala
/// com este autor (`nas-history::Author`), nunca hardcoded como
/// `User`.
pub struct CommandContext<'a> {
    pub workspace: &'a mut Workspace,
    pub author: Author,
    /// Efeitos já aprovados pra esta chamada específica — quem monta
    /// o `CommandContext` (hoje, `nas-mcp`) decide o que entra aqui,
    /// consultando a política de permissões do projeto
    /// (`nas_history::load_permissions`, ADR 0006 §5, etapa 3d) antes
    /// de chamar `Registry::run`; sem o efeito aqui, um comando
    /// `WriteFile`/`Shell`/`Network` pedido por um agente de IA é
    /// recusado (`CommandError::NeedsApproval`).
    pub approved_effects: std::collections::HashSet<crate::effect::Effect>,
}
