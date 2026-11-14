//! Spike: prova de que helix-core (tag 25.07.1) dá para usar como biblioteca.
//!
//! Abre um arquivo num Rope, aplica duas Transactions (inserir e apagar) e
//! desfaz as duas com History, imprimindo o estado do buffer a cada passo.

use helix_core::{Rope, Selection, Transaction, history::History, history::State};
use std::fs;
use std::io::BufReader;

fn print_step(label: &str, doc: &Rope) {
    println!("--- {label} ---");
    println!("{doc}");
    println!("(len_chars = {})", doc.len_chars());
}

fn main() -> std::io::Result<()> {
    let path = "sample.txt";
    fs::write(path, "ola mundo\n")?;

    let file = fs::File::open(path)?;
    let mut doc = Rope::from_reader(BufReader::new(file))?;
    print_step("abriu arquivo", &doc);

    let mut history = History::default();

    // 1) inserir "lindo " antes de "mundo"
    let selection = Selection::point(4);
    let original = State {
        doc: doc.clone(),
        selection: selection.clone(),
    };
    let insert_tx = Transaction::insert(&doc, &selection, "lindo ".into());
    insert_tx.apply(&mut doc);
    history.commit_revision(&insert_tx, &original);
    print_step("depois de inserir 'lindo '", &doc);

    // 2) apagar "ola " do início
    let selection_before_delete = Selection::point(0);
    let original = State {
        doc: doc.clone(),
        selection: selection_before_delete,
    };
    let delete_tx = Transaction::delete(&doc, [(0usize, 4usize)].into_iter());
    delete_tx.apply(&mut doc);
    history.commit_revision(&delete_tx, &original);
    print_step("depois de apagar 'ola '", &doc);

    // 3) desfazer o apagar
    if let Some(inversion) = history.undo() {
        inversion.clone().apply(&mut doc);
    }
    print_step("depois do 1o undo (desfez o apagar)", &doc);

    // 4) desfazer o inserir
    if let Some(inversion) = history.undo() {
        inversion.clone().apply(&mut doc);
    }
    print_step("depois do 2o undo (desfez o inserir)", &doc);

    fs::remove_file(path)?;
    Ok(())
}
