//! In-memory library/module DAG. See `specs/05-library-module-graph.md` §7.
//!
//! Resolution of `use`/`import`/`export` clauses is stubbed pending Sprint 04
//! `define module` / `define library` parsing.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::lid::Lid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LibraryId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ModuleId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BindingId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Symbol(pub u32);

#[derive(Debug, Default)]
pub struct SymbolInterner {
    symbols: Vec<String>,
    index: HashMap<String, Symbol>,
}

impl SymbolInterner {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn intern(&mut self, s: &str) -> Symbol {
        if let Some(&id) = self.index.get(s) {
            return id;
        }
        let id = Symbol(self.symbols.len() as u32);
        self.symbols.push(s.to_string());
        self.index.insert(s.to_string(), id);
        id
    }

    pub fn resolve(&self, sym: Symbol) -> &str {
        &self.symbols[sym.0 as usize]
    }
}

#[derive(Debug, Clone)]
pub enum LibraryRef {
    Resolved(LibraryId),
    Unresolved(Symbol),
}

#[derive(Debug, Clone)]
pub enum ModuleRef {
    Resolved(ModuleId),
    Unresolved {
        library: Option<Symbol>,
        module: Symbol,
    },
}

#[derive(Debug, Clone)]
pub struct LibraryUse {
    pub library: LibraryRef,
    pub imported_modules: Option<Vec<Symbol>>,
    pub reexported_modules: Vec<Symbol>,
}

#[derive(Debug, Clone)]
pub enum Import {
    All,
    Listed(Vec<Symbol>),
}

#[derive(Debug, Clone)]
pub enum Reexport {
    None,
    All,
    Listed(Vec<Symbol>),
}

#[derive(Debug, Clone)]
pub struct ModuleUse {
    pub module: ModuleRef,
    pub import: Import,
    pub exclude: Vec<Symbol>,
    pub rename: Vec<(Symbol, Symbol)>,
    pub prefix: Option<String>,
    pub reexport: Reexport,
}

#[derive(Debug, Clone)]
pub struct Library {
    pub id: LibraryId,
    pub name: Symbol,
    pub uses: Vec<LibraryUse>,
    pub modules: Vec<ModuleId>,
    pub exports: Vec<ModuleId>,
    pub source_lid: PathBuf,
    pub source_package_json: Option<PathBuf>,
    pub source_library_dylan: Option<PathBuf>,
    pub files: Vec<PathBuf>,
    pub generation: u64,
}

#[derive(Debug, Clone)]
pub struct Module {
    pub id: ModuleId,
    pub library: LibraryId,
    pub name: Symbol,
    pub uses: Vec<ModuleUse>,
    pub creates: Vec<Symbol>,
    pub exports: Vec<Symbol>,
    pub bindings: HashMap<Symbol, BindingId>,
    pub source_files: Vec<PathBuf>,
    pub generation: u64,
}

#[derive(Debug, Default)]
pub struct Graph {
    libraries: Vec<Library>,
    modules: Vec<Module>,
    interner: SymbolInterner,
}

impl Graph {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_library_from_lid(&mut self, lid: &Lid) -> LibraryId {
        let name_str = lid.library.clone().unwrap_or_default();
        let name = self.interner.intern(&name_str);
        let id = LibraryId(self.libraries.len() as u32);
        let lid_dir = lid.path.parent().map(Path::to_path_buf);
        let files = lid
            .files
            .iter()
            .map(|f| {
                let with_ext = if f.ends_with(".dylan") {
                    f.clone()
                } else {
                    format!("{f}.dylan")
                };
                match &lid_dir {
                    Some(d) => d.join(with_ext),
                    None => PathBuf::from(with_ext),
                }
            })
            .collect();
        self.libraries.push(Library {
            id,
            name,
            uses: Vec::new(),
            modules: Vec::new(),
            exports: Vec::new(),
            source_lid: lid.path.clone(),
            source_package_json: None,
            source_library_dylan: None,
            files,
            generation: 0,
        });
        id
    }

    pub fn add_module(&mut self, library: LibraryId, name: &str) -> ModuleId {
        let sym = self.interner.intern(name);
        let id = ModuleId(self.modules.len() as u32);
        self.modules.push(Module {
            id,
            library,
            name: sym,
            uses: Vec::new(),
            creates: Vec::new(),
            exports: Vec::new(),
            bindings: HashMap::new(),
            source_files: Vec::new(),
            generation: 0,
        });
        self.libraries[library.0 as usize].modules.push(id);
        id
    }

    pub fn intern(&mut self, s: &str) -> Symbol {
        self.interner.intern(s)
    }

    pub fn resolve(&self, sym: Symbol) -> &str {
        self.interner.resolve(sym)
    }

    pub fn library(&self, id: LibraryId) -> &Library {
        &self.libraries[id.0 as usize]
    }

    pub fn module(&self, id: ModuleId) -> &Module {
        &self.modules[id.0 as usize]
    }

    pub fn libraries(&self) -> impl Iterator<Item = &Library> {
        self.libraries.iter()
    }

    pub fn modules(&self) -> impl Iterator<Item = &Module> {
        self.modules.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lid::parse_lid_str;

    #[test]
    fn build_library_files_have_ext() {
        let lid = parse_lid_str(
            "library: x\nfiles: a b sub/c\n",
            PathBuf::from("/tmp/x.lid"),
        );
        let mut g = Graph::new();
        let id = g.add_library_from_lid(&lid);
        let lib = g.library(id);
        assert_eq!(lib.files.len(), 3);
        assert!(lib.files[0].to_string_lossy().ends_with("a.dylan"));
        assert!(lib.files[2].to_string_lossy().ends_with("c.dylan"));
    }

    #[test]
    fn module_attaches_to_library() {
        let lid = parse_lid_str("library: x\nfiles: a\n", PathBuf::from("x.lid"));
        let mut g = Graph::new();
        let lib = g.add_library_from_lid(&lid);
        let m = g.add_module(lib, "internal");
        assert_eq!(g.library(lib).modules, vec![m]);
        let sym = g.module(m).name;
        assert_eq!(g.resolve(sym), "internal");
    }
}
