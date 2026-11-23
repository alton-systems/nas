use std::collections::HashMap;
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

    /// Registra uma mudança de texto no journal (ADR 0005 §1/§2) —
    /// sem efeito se não houver journal configurado (ex.: num
    /// `Workspace` de teste que não precisa de histórico), ou se a
    /// mudança for vazia (nada a registrar). Também marca `file`
    /// como rastreado na primeira vez (`nas_history::ensure_tracked`),
    /// igual a `App::journal_record_for` em `nas-term`.
    pub fn record_change(
        &mut self,
        author: Author,
        command: &str,
        file: &str,
        change: &TextChange,
    ) -> Result<(), CommandError> {
        if change.from == change.to && change.removed.is_empty() && change.inserted.is_empty() {
            return Ok(());
        }
        if let Some(history_dir) = &self.history_dir {
            let disk_content = std::fs::read(file).ok();
            let _ = nas_history::ensure_tracked(history_dir, file, disk_content.as_deref());
        }
        if let Some(journal) = &mut self.journal {
            journal.record(
                nas_history::now_unix(),
                author,
                command.to_string(),
                file.to_string(),
                change.from,
                change.to,
                change.removed.clone(),
                change.inserted.clone(),
            )?;
        }
        Ok(())
    }

    pub fn document(&self, path: &Path) -> Option<&Document> {
        self.documents.get(path)
    }

    pub fn document_mut(&mut self, path: &Path) -> Option<&mut Document> {
        self.documents.get_mut(path)
    }

    /// Abre `path` como documento, se ainda não estiver — idempotente:
    /// chamar de novo com o mesmo caminho não reabre (perderia
    /// alterações em memória não salvas).
    pub fn open(&mut self, path: impl AsRef<Path>) -> Result<&Document, CommandError> {
        let path = path.as_ref().to_path_buf();
        if !self.documents.contains_key(&path) {
            let document = Document::open(&path)?;
            self.documents.insert(path.clone(), document);
        }
        Ok(self.documents.get(&path).expect("acabou de ser inserido"))
    }

    /// Cria um documento novo, vazio, em `path` — pra `create_file`.
    /// Diferente de `open`: recusa se `path` já existir no disco ou
    /// já estiver aberto, pra `create_file` nunca sobrescrever algo
    /// sem querer (quem quer sobrescrever usa `edit`/`open_file`).
    pub fn create(&mut self, path: impl AsRef<Path>) -> Result<(), CommandError> {
        let path = path.as_ref().to_path_buf();
        if path.exists() || self.documents.contains_key(&path) {
            return Err(CommandError::Io(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                format!("{} já existe", path.display()),
            )));
        }
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, "")?;
        let document = Document::open(&path)?;
        self.documents.insert(path, document);
        Ok(())
    }

    /// O conteúdo atual de um caminho rastreado — do documento aberto,
    /// se houver (pode ter alterações em memória ainda não salvas),
    /// senão do disco. Usado por `checkpoint_create`/a restauração,
    /// que precisam do estado de **todo** arquivo rastreado (ADR 0005
    /// §4), não só dos documentos abertos.
    pub fn current_content_for(&self, path: &str) -> Option<Vec<u8>> {
        if let Some(document) = self.documents.get(Path::new(path)) {
            return Some(document.full_text().into_bytes());
        }
        std::fs::read(path).ok()
    }

    /// Escreve `content` em `path`: se já há um documento aberto pra
    /// esse caminho, substitui o conteúdo dele (`replace_all`, que
    /// entra no desfazer por tecla) e salva; senão, abre um documento
    /// descartável só pra escrever — restaurar/recriar afeta o
    /// arquivo, não só um documento que porventura esteja aberto.
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
}

/// O que um comando recebe pra agir: o `Workspace` (§ acima) mais
/// quem está chamando — todo comando que muda texto/disco journala
/// com este autor (`nas-history::Author`), nunca hardcoded como
/// `User`.
pub struct CommandContext<'a> {
    pub workspace: &'a mut Workspace,
    pub author: Author,
}
