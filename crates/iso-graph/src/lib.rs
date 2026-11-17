//! Graph algorithms over small integer node ids: layers by longest path, cycles by strongly
//! connected components, impact sets, connected components and module clustering.
//!
//! Callers map their own ids (symbols, files, modules) to `0..n` and back.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

/// A directed graph as adjacency lists. Duplicate edges are allowed and ignored by algorithms.
#[derive(Clone, Debug, Default)]
pub struct Graph {
    pub out: Vec<Vec<usize>>,
}

impl Graph {
    pub fn new(n: usize) -> Self {
        Graph {
            out: vec![Vec::new(); n],
        }
    }
    pub fn len(&self) -> usize {
        self.out.len()
    }
    pub fn is_empty(&self) -> bool {
        self.out.is_empty()
    }
    pub fn add_edge(&mut self, a: usize, b: usize) {
        self.out[a].push(b);
    }
    pub fn reversed(&self) -> Graph {
        let mut g = Graph::new(self.len());
        for (a, outs) in self.out.iter().enumerate() {
            for &b in outs {
                g.out[b].push(a);
            }
        }
        g
    }
}

/// Strongly connected components (Tarjan, iterative). Components come out in reverse
/// topological order: every edge goes from a later component to an earlier one or stays inside.
pub fn scc(g: &Graph) -> Vec<Vec<usize>> {
    let n = g.len();
    let mut index = vec![usize::MAX; n];
    let mut low = vec![0usize; n];
    let mut on_stack = vec![false; n];
    let mut stack = Vec::new();
    let mut comps = Vec::new();
    let mut next = 0usize;
    for start in 0..n {
        if index[start] != usize::MAX {
            continue;
        }
        // (node, next child position)
        let mut call: Vec<(usize, usize)> = vec![(start, 0)];
        index[start] = next;
        low[start] = next;
        next += 1;
        stack.push(start);
        on_stack[start] = true;
        while let Some(&mut (v, ref mut pos)) = call.last_mut() {
            if *pos < g.out[v].len() {
                let w = g.out[v][*pos];
                *pos += 1;
                if index[w] == usize::MAX {
                    index[w] = next;
                    low[w] = next;
                    next += 1;
                    stack.push(w);
                    on_stack[w] = true;
                    call.push((w, 0));
                } else if on_stack[w] {
                    low[v] = low[v].min(index[w]);
                }
            } else {
                call.pop();
                if let Some(&(parent, _)) = call.last() {
                    low[parent] = low[parent].min(low[v]);
                }
                if low[v] == index[v] {
                    let mut comp = Vec::new();
                    loop {
                        let w = stack.pop().expect("stack");
                        on_stack[w] = false;
                        comp.push(w);
                        if w == v {
                            break;
                        }
                    }
                    comp.sort_unstable();
                    comps.push(comp);
                }
            }
        }
    }
    comps
}

/// Cycles: components with more than one node, or a node with a self edge.
pub fn cycles(g: &Graph) -> Vec<Vec<usize>> {
    scc(g)
        .into_iter()
        .filter(|c| c.len() > 1 || g.out[c[0]].contains(&c[0]))
        .collect()
}

/// Layer of every node = length of the longest path to a leaf, with each strongly connected
/// component collapsed to one node (so recursion does not make levels unbounded). Leaves are 0;
/// entry points have the highest level.
pub fn layers(g: &Graph) -> Vec<u32> {
    let comps = scc(g);
    let mut comp_of = vec![0usize; g.len()];
    for (ci, c) in comps.iter().enumerate() {
        for &v in c {
            comp_of[v] = ci;
        }
    }
    // Tarjan emits components in reverse topological order, so callees come first.
    let mut level = vec![0u32; comps.len()];
    for (ci, c) in comps.iter().enumerate() {
        let mut best = 0u32;
        let mut has_out = false;
        for &v in c {
            for &w in &g.out[v] {
                let cw = comp_of[w];
                if cw != ci {
                    has_out = true;
                    best = best.max(level[cw] + 1);
                }
            }
        }
        level[ci] = if has_out { best } else { 0 };
    }
    (0..g.len()).map(|v| level[comp_of[v]]).collect()
}

/// Nodes reachable from `start` (excluding it) with their hop distance. `transitive = false`
/// returns direct neighbors only.
pub fn reach(g: &Graph, start: usize, transitive: bool) -> Vec<(usize, u32)> {
    let mut seen = HashSet::from([start]);
    let mut out = Vec::new();
    let mut q = VecDeque::from([(start, 0u32)]);
    while let Some((v, d)) = q.pop_front() {
        if d >= 1 && !transitive {
            continue;
        }
        for &w in &g.out[v] {
            if seen.insert(w) {
                out.push((w, d + 1));
                q.push_back((w, d + 1));
            }
        }
    }
    out
}

/// Weakly connected components, as a component index per node.
pub fn components(g: &Graph) -> Vec<usize> {
    let rev = g.reversed();
    let mut comp = vec![usize::MAX; g.len()];
    let mut c = 0;
    for s in 0..g.len() {
        if comp[s] != usize::MAX {
            continue;
        }
        let mut stack = vec![s];
        comp[s] = c;
        while let Some(v) = stack.pop() {
            for &w in g.out[v].iter().chain(rev.out[v].iter()) {
                if comp[w] == usize::MAX {
                    comp[w] = c;
                    stack.push(w);
                }
            }
        }
        c += 1;
    }
    comp
}

// ---------------------------------------------------------------- module clustering

/// Accessible palette for module colors: distinct hues at similar lightness, readable as a dot
/// or a stripe on both themes. The UI lightens them for dark mode.
pub const PALETTE: &[&str] = &[
    "#2f6fb5", "#b0417a", "#2f7d32", "#b06800", "#6a4ea6", "#0b7285", "#a63d2a", "#5b6b00",
    "#8a4fbf", "#00796b", "#9c5b00", "#3a5ba0", "#7a6000", "#ad3060", "#4a7a8c", "#6d6d00",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModuleCandidate {
    /// Stable id: the directory path ("" → "root").
    pub id: String,
    pub dir: String,
    pub name: String,
    pub files: Vec<usize>,
    pub color: String,
}

/// Options for [`cluster`].
#[derive(Clone, Copy, Debug)]
pub struct ClusterOptions {
    /// A directory with more files than this is split into its subdirectories.
    pub max_files: usize,
    /// Modules smaller than this merge into the neighbor they import from most, or their parent.
    pub min_files: usize,
}

impl Default for ClusterOptions {
    fn default() -> Self {
        ClusterOptions {
            max_files: 120,
            min_files: 3,
        }
    }
}

/// Module candidates from the directory tree combined with the file import graph.
///
/// 1. Split the tree top-down: a directory becomes a module when it holds at most `max_files`
///    files (recursively) or has no subdirectories; otherwise its subdirectories are visited
///    and its own loose files form a module of their own.
/// 2. Merge modules smaller than `min_files` into the module they share the most import edges
///    with (a one-step community move), falling back to the parent directory's module.
/// 3. Color greedily so modules that import each other get different palette entries.
///
/// `paths[i]` is file i's repo-relative path; `edges` are file-level imports.
pub fn cluster(
    paths: &[&str],
    edges: &[(usize, usize)],
    opts: ClusterOptions,
) -> Vec<ModuleCandidate> {
    // Directory → files beneath it (recursive count) and direct files.
    let mut direct: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    let mut total: HashMap<String, usize> = HashMap::new();
    let mut subdirs: BTreeMap<String, HashSet<String>> = BTreeMap::new();
    for (i, p) in paths.iter().enumerate() {
        let dir = p.rsplit_once('/').map(|(d, _)| d).unwrap_or("").to_string();
        direct.entry(dir.clone()).or_default().push(i);
        let mut d = dir;
        loop {
            *total.entry(d.clone()).or_default() += 1;
            if d.is_empty() {
                break;
            }
            let parent = d.rsplit_once('/').map(|(a, _)| a).unwrap_or("").to_string();
            subdirs.entry(parent.clone()).or_default().insert(d.clone());
            d = parent;
        }
    }
    let mut modules: Vec<(String, Vec<usize>)> = Vec::new();
    let mut stack = vec![String::new()];
    while let Some(dir) = stack.pop() {
        let subs = subdirs.get(&dir).cloned().unwrap_or_default();
        let n = total.get(&dir).copied().unwrap_or(0);
        if n == 0 {
            continue;
        }
        if n <= opts.max_files || subs.is_empty() {
            let mut files = Vec::new();
            collect_under(&dir, &direct, &subdirs, &mut files);
            modules.push((dir, files));
        } else {
            if let Some(own) = direct.get(&dir) {
                modules.push((dir.clone(), own.clone()));
            }
            let mut subs: Vec<String> = subs.into_iter().collect();
            subs.sort();
            stack.extend(subs.into_iter().rev());
        }
    }
    modules.sort_by(|a, b| a.0.cmp(&b.0));

    // Merge tiny modules.
    let mut module_of = vec![usize::MAX; paths.len()];
    for (mi, (_, files)) in modules.iter().enumerate() {
        for &f in files {
            module_of[f] = mi;
        }
    }
    let mut alive: Vec<bool> = vec![true; modules.len()];
    let dir_index: HashMap<String, usize> = modules
        .iter()
        .enumerate()
        .map(|(i, (d, _))| (d.clone(), i))
        .collect();
    // Process smallest first, deterministic.
    let mut order: Vec<usize> = (0..modules.len()).collect();
    order.sort_by_key(|&i| (modules[i].1.len(), modules[i].0.clone()));
    for mi in order {
        if modules.len() <= 1 || modules[mi].1.len() >= opts.min_files || !alive[mi] {
            continue;
        }
        let mut weight: BTreeMap<usize, usize> = BTreeMap::new();
        for &(a, b) in edges {
            let (ma, mb) = (module_of[a], module_of[b]);
            if ma == mi && mb != mi && alive[mb] {
                *weight.entry(mb).or_default() += 1;
            } else if mb == mi && ma != mi && alive[ma] {
                *weight.entry(ma).or_default() += 1;
            }
        }
        let target = weight
            .iter()
            .max_by_key(|(k, w)| (**w, std::cmp::Reverse(**k)))
            .map(|(k, _)| *k)
            .or_else(|| {
                // Nearest ancestor directory that is a live module.
                let mut d = modules[mi].0.clone();
                while !d.is_empty() {
                    d = d.rsplit_once('/').map(|(a, _)| a).unwrap_or("").to_string();
                    if let Some(&t) = dir_index.get(&d)
                        && t != mi
                        && alive[t]
                    {
                        return Some(t);
                    }
                }
                None
            });
        if let Some(t) = target {
            let files = std::mem::take(&mut modules[mi].1);
            for &f in &files {
                module_of[f] = t;
            }
            modules[t].1.extend(files);
            alive[mi] = false;
        }
    }
    let mut out: Vec<ModuleCandidate> = modules
        .into_iter()
        .zip(alive)
        .filter(|(_, a)| *a)
        .map(|((dir, mut files), _)| {
            files.sort_unstable();
            ModuleCandidate {
                id: if dir.is_empty() {
                    "root".into()
                } else {
                    dir.clone()
                },
                name: module_name(&dir),
                dir,
                files,
                color: String::new(),
            }
        })
        .collect();
    // Greedy coloring over the module graph, largest modules first.
    let idx: HashMap<usize, usize> = out
        .iter()
        .enumerate()
        .flat_map(|(mi, m)| m.files.iter().map(move |&f| (f, mi)))
        .collect();
    let mut adj: Vec<HashSet<usize>> = vec![HashSet::new(); out.len()];
    for &(a, b) in edges {
        if let (Some(&ma), Some(&mb)) = (idx.get(&a), idx.get(&b))
            && ma != mb
        {
            adj[ma].insert(mb);
            adj[mb].insert(ma);
        }
    }
    let mut order: Vec<usize> = (0..out.len()).collect();
    order.sort_by_key(|&i| (std::cmp::Reverse(out[i].files.len()), out[i].dir.clone()));
    let mut color_ix: Vec<Option<usize>> = vec![None; out.len()];
    for (rank, &mi) in order.iter().enumerate() {
        let used: HashSet<usize> = adj[mi].iter().filter_map(|&n| color_ix[n]).collect();
        let start = rank % PALETTE.len();
        let pick = (0..PALETTE.len())
            .map(|k| (start + k) % PALETTE.len())
            .find(|c| !used.contains(c))
            .unwrap_or(start);
        color_ix[mi] = Some(pick);
    }
    for (mi, m) in out.iter_mut().enumerate() {
        m.color = PALETTE[color_ix[mi].unwrap_or(0)].to_string();
    }
    out
}

fn collect_under(
    dir: &str,
    direct: &BTreeMap<String, Vec<usize>>,
    subdirs: &BTreeMap<String, HashSet<String>>,
    out: &mut Vec<usize>,
) {
    if let Some(f) = direct.get(dir) {
        out.extend(f);
    }
    if let Some(s) = subdirs.get(dir) {
        for d in s {
            collect_under(d, direct, subdirs, out);
        }
    }
}

/// Default module name from its directory: the last one or two path segments.
pub fn module_name(dir: &str) -> String {
    if dir.is_empty() {
        return "Repository root".into();
    }
    let segs: Vec<&str> = dir.split('/').collect();
    let last = segs[segs.len() - 1];
    let generic = [
        "src", "lib", "app", "pkg", "internal", "core", "utils", "util", "common", "test", "tests",
    ];
    if segs.len() >= 2 && generic.contains(&last) {
        format!("{}/{}", segs[segs.len() - 2], last)
    } else {
        last.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g(n: usize, e: &[(usize, usize)]) -> Graph {
        let mut g = Graph::new(n);
        for &(a, b) in e {
            g.add_edge(a, b);
        }
        g
    }

    #[test]
    fn layers_by_longest_path() {
        // 0 → 1 → 2, 0 → 2, 3 alone
        let gr = g(4, &[(0, 1), (1, 2), (0, 2)]);
        assert_eq!(layers(&gr), vec![2, 1, 0, 0]);
    }

    #[test]
    fn cycles_collapse_and_are_reported() {
        // 0 → 1 → 2 → 1, 2 → 3
        let gr = g(4, &[(0, 1), (1, 2), (2, 1), (2, 3)]);
        let l = layers(&gr);
        assert_eq!(l[3], 0);
        assert_eq!(l[1], l[2]);
        assert_eq!(l[1], 1);
        assert_eq!(l[0], 2);
        assert_eq!(cycles(&gr), vec![vec![1, 2]]);
        let self_loop = g(2, &[(0, 0), (0, 1)]);
        assert_eq!(cycles(&self_loop), vec![vec![0]]);
    }

    #[test]
    fn reach_direct_and_transitive() {
        let gr = g(4, &[(0, 1), (1, 2), (2, 3)]);
        assert_eq!(reach(&gr, 0, false), vec![(1, 1)]);
        assert_eq!(reach(&gr, 0, true), vec![(1, 1), (2, 2), (3, 3)]);
        assert_eq!(reach(&gr.reversed(), 3, true), vec![(2, 1), (1, 2), (0, 3)]);
    }

    #[test]
    fn components_split() {
        let gr = g(5, &[(0, 1), (2, 3)]);
        let c = components(&gr);
        assert_eq!(c[0], c[1]);
        assert_eq!(c[2], c[3]);
        assert_ne!(c[0], c[2]);
        assert_ne!(c[4], c[0]);
    }

    #[test]
    fn clustering_is_stable_and_merges_tiny_modules() {
        let mut paths = vec![];
        for i in 0..10 {
            paths.push(format!("src/api/f{i}.ts"));
        }
        for i in 0..10 {
            paths.push(format!("src/db/f{i}.ts"));
        }
        paths.push("src/misc/one.ts".to_string());
        paths.push("README.md".to_string());
        let p: Vec<&str> = paths.iter().map(|s| s.as_str()).collect();
        let edges = vec![(20, 0), (0, 10)];
        let opts = ClusterOptions {
            max_files: 12,
            min_files: 2,
        };
        let a = cluster(&p, &edges, opts);
        let b = cluster(&p, &edges, opts);
        assert_eq!(a, b);
        let ids: Vec<&str> = a.iter().map(|m| m.id.as_str()).collect();
        assert!(
            ids.contains(&"src/api") && ids.contains(&"src/db"),
            "{ids:?}"
        );
        // src/misc (1 file) merges into src/api because it imports it.
        let api = a.iter().find(|m| m.id == "src/api").unwrap();
        assert!(api.files.contains(&20));
        // Adjacent modules get different colors.
        let db = a.iter().find(|m| m.id == "src/db").unwrap();
        assert_ne!(api.color, db.color);
        assert_eq!(module_name("crates/iso-core/src"), "iso-core/src");
    }
}
