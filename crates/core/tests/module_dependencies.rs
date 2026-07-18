//! Cooperative dependency-law check for explicit Rust paths.
//!
//! `syn` sees ordinary paths, imports, aliases, and re-exports. Macro expansion
//! and rustc name resolution remain review responsibilities by design.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use syn::visit::{self, Visit};
use syn::{ItemMod, ItemUse, UseTree};

#[test]
fn core_module_dependencies_follow_the_architecture_dag() {
    let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut violations = BTreeSet::new();

    for file in rust_files(&source_root) {
        let module = module_path(&source_root, &file);
        let Some(owner) = module.first() else {
            continue;
        };
        let source = fs::read_to_string(&file).expect("a core source file must be readable");
        let syntax = syn::parse_file(&source).expect("a core source file must parse as Rust");
        let mut visitor = DependencyVisitor {
            module: &module,
            owner,
            file: &file,
            violations: &mut violations,
            inline_depth: 0,
        };

        visitor.visit_file(&syntax);
    }

    assert!(
        violations.is_empty(),
        "core module dependency violations:\n{}",
        violations.into_iter().collect::<Vec<_>>().join("\n"),
    );
}

#[test]
fn dependency_resolution_distinguishes_allowed_and_forbidden_siblings() {
    let syntax_module = vec![String::from("syntax"), String::from("parser")];
    let compiler_module = vec![String::from("compiler"), String::from("parse")];
    let render_graph_module = vec![String::from("render_graph")];
    let diagnostics = vec![String::from("crate"), String::from("diagnostics")];
    let model = vec![String::from("crate"), String::from("model")];
    let graph_timeline = vec![String::from("crate"), String::from("timeline")];
    let timeline = vec![
        String::from("super"),
        String::from("super"),
        String::from("timeline"),
    ];

    let syntax_to_diagnostics = resolve_owner(&syntax_module, &diagnostics, 0)
        .expect("the explicit crate path has an owner");
    let syntax_to_model =
        resolve_owner(&syntax_module, &model, 0).expect("the explicit crate path has an owner");
    let compiler_to_timeline = resolve_owner(&compiler_module, &timeline, 0)
        .expect("the explicit ancestor path has an owner");
    let graph_to_timeline = resolve_owner(&render_graph_module, &graph_timeline, 0)
        .expect("the explicit crate path has an owner");
    let graph_inline_test = vec![String::from("super"), String::from("RenderGraph")];
    let graph_to_itself = resolve_owner(&render_graph_module, &graph_inline_test, 1)
        .expect("the inline test path retains its file owner");

    assert!(!dependency_allowed("syntax", syntax_to_diagnostics));
    assert!(dependency_allowed("syntax", syntax_to_model));
    assert!(dependency_allowed("compiler", compiler_to_timeline));
    assert!(dependency_allowed("render_graph", graph_to_timeline));
    assert_eq!(graph_to_itself, "render_graph");
}

struct DependencyVisitor<'a> {
    module: &'a [String],
    owner: &'a str,
    file: &'a Path,
    violations: &'a mut BTreeSet<String>,
    inline_depth: usize,
}

impl<'ast> Visit<'ast> for DependencyVisitor<'_> {
    fn visit_path(&mut self, path: &'ast syn::Path) {
        let segments = path
            .segments
            .iter()
            .map(|segment| segment.ident.to_string())
            .collect::<Vec<_>>();
        self.check_path(&segments);
        visit::visit_path(self, path);
    }

    fn visit_item_use(&mut self, item: &'ast ItemUse) {
        for path in use_paths(Vec::new(), &item.tree) {
            self.check_path(&path);
        }

        visit::visit_item_use(self, item);
    }

    fn visit_item_mod(&mut self, item: &'ast ItemMod) {
        let is_inline = item.content.is_some();
        self.inline_depth += usize::from(is_inline);
        visit::visit_item_mod(self, item);
        self.inline_depth -= usize::from(is_inline);
    }
}

impl DependencyVisitor<'_> {
    fn check_path(&mut self, path: &[String]) {
        let Some(target) = resolve_owner(self.module, path, self.inline_depth) else {
            return;
        };

        if target == self.owner || dependency_allowed(self.owner, target) {
            return;
        }

        self.violations.insert(format!(
            "{}: {} may not depend on {} through {}",
            self.file.display(),
            self.owner,
            target,
            path.join("::"),
        ));
    }
}

fn rust_files(directory: &Path) -> Vec<PathBuf> {
    let entries = fs::read_dir(directory).expect("the core source directory must be readable");
    let mut files = Vec::new();

    for entry in entries {
        let path = entry
            .expect("a source directory entry must be readable")
            .path();

        if path.is_dir() {
            files.extend(rust_files(&path));
            continue;
        }

        if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }

    files.sort();
    files
}

fn module_path(source_root: &Path, file: &Path) -> Vec<String> {
    let relative = file
        .strip_prefix(source_root)
        .expect("a collected source file must be below the source root");
    let mut components = relative
        .parent()
        .into_iter()
        .flat_map(Path::components)
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    let stem = relative
        .file_stem()
        .expect("a Rust source file must have a stem")
        .to_string_lossy();

    if stem != "lib" && stem != "mod" {
        components.push(stem.into_owned());
    }

    components
}

fn use_paths(prefix: Vec<String>, tree: &UseTree) -> Vec<Vec<String>> {
    match tree {
        UseTree::Path(path) => {
            let mut prefix = prefix;
            prefix.push(path.ident.to_string());
            use_paths(prefix, &path.tree)
        }
        UseTree::Name(name) => {
            let mut path = prefix;
            path.push(name.ident.to_string());
            vec![path]
        }
        UseTree::Rename(rename) => {
            let mut path = prefix;
            path.push(rename.ident.to_string());
            vec![path]
        }
        UseTree::Glob(_) => vec![prefix],
        UseTree::Group(group) => {
            let mut paths = Vec::new();

            for item in &group.items {
                paths.extend(use_paths(prefix.clone(), item));
            }

            paths
        }
    }
}

fn resolve_owner<'a>(
    module: &'a [String],
    path: &'a [String],
    inline_depth: usize,
) -> Option<&'a str> {
    let first = path.first()?.as_str();

    if first == "crate" {
        return path.get(1).map(String::as_str);
    }

    if first == "self" {
        return module.first().map(String::as_str);
    }

    if first != "super" {
        return None;
    }

    let parent_count = path
        .iter()
        .take_while(|segment| segment.as_str() == "super")
        .count();
    let retained = module
        .len()
        .saturating_add(inline_depth)
        .saturating_sub(parent_count);

    if retained > 0 {
        module.first().map(String::as_str)
    } else {
        path.get(parent_count).map(String::as_str)
    }
}

fn dependency_allowed(owner: &str, target: &str) -> bool {
    match owner {
        "model" => false,
        "syntax" | "diagnostics" | "timeline" => target == "model",
        "render_graph" => matches!(target, "model" | "timeline"),
        "compiler" => matches!(target, "model" | "syntax" | "diagnostics" | "timeline"),
        "protocol" => matches!(target, "model" | "diagnostics" | "timeline"),
        _ => false,
    }
}
