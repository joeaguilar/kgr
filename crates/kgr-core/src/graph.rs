use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::algo::tarjan_scc;
use petgraph::Direction;

use crate::types::{DepEdge, DepGraph, FileNode, ImportKind};

pub struct KGraph {
    inner: DiGraph<PathBuf, ImportKind>,
    node_index: HashMap<PathBuf, NodeIndex>,
}

impl KGraph {
    pub fn from_files(files: &[FileNode]) -> Self {
        let mut graph = DiGraph::new();
        let mut node_index = HashMap::new();

        // Add all files as nodes
        for file in files {
            let idx = graph.add_node(file.path.clone());
            node_index.insert(file.path.clone(), idx);
        }

        // Add edges for resolved imports
        for file in files {
            if let Some(&from_idx) = node_index.get(&file.path) {
                for import in &file.imports {
                    if let Some(ref resolved) = import.resolved {
                        if let Some(&to_idx) = node_index.get(resolved) {
                            graph.add_edge(from_idx, to_idx, import.kind.clone());
                        }
                    }
                }
            }
        }

        Self {
            inner: graph,
            node_index,
        }
    }

    pub fn cycles(&self) -> Vec<Vec<PathBuf>> {
        tarjan_scc(&self.inner)
            .into_iter()
            .filter(|scc| scc.len() > 1)
            .map(|scc| {
                scc.into_iter()
                    .map(|idx| self.inner[idx].clone())
                    .collect()
            })
            .collect()
    }

    pub fn roots(&self) -> Vec<PathBuf> {
        self.node_index
            .iter()
            .filter(|(_, &idx)| {
                self.inner
                    .neighbors_directed(idx, Direction::Incoming)
                    .next()
                    .is_none()
            })
            .map(|(path, _)| path.clone())
            .collect()
    }

    pub fn orphans(&self) -> Vec<PathBuf> {
        self.node_index
            .iter()
            .filter(|(_, &idx)| {
                let no_in = self
                    .inner
                    .neighbors_directed(idx, Direction::Incoming)
                    .next()
                    .is_none();
                let no_out = self
                    .inner
                    .neighbors_directed(idx, Direction::Outgoing)
                    .next()
                    .is_none();
                no_in && no_out
            })
            .map(|(path, _)| path.clone())
            .collect()
    }

    pub fn to_dep_graph(&self, root: PathBuf, files: Vec<FileNode>) -> DepGraph {
        let mut edges = Vec::new();
        for edge in self.inner.edge_indices() {
            let (from_idx, to_idx) = self.inner.edge_endpoints(edge).unwrap();
            let kind = self.inner[edge].clone();
            edges.push(DepEdge {
                from: self.inner[from_idx].clone(),
                to: self.inner[to_idx].clone(),
                kind,
            });
        }

        let cycles = self.cycles();
        let mut roots = self.roots();
        roots.sort();
        let mut orphans = self.orphans();
        orphans.sort();

        DepGraph {
            root,
            files,
            edges,
            cycles,
            roots,
            orphans,
        }
    }

    pub fn cycle_edges(&self) -> Vec<(PathBuf, PathBuf)> {
        let sccs = tarjan_scc(&self.inner);
        let mut cycle_edges = Vec::new();

        for scc in &sccs {
            if scc.len() <= 1 {
                continue;
            }
            let scc_set: std::collections::HashSet<_> = scc.iter().copied().collect();
            for &node in scc {
                for neighbor in self.inner.neighbors_directed(node, Direction::Outgoing) {
                    if scc_set.contains(&neighbor) {
                        cycle_edges.push((
                            self.inner[node].clone(),
                            self.inner[neighbor].clone(),
                        ));
                    }
                }
            }
        }

        cycle_edges
    }

    /// All transitive dependencies of a node (BFS)
    pub fn transitive_deps(&self, from: &Path, max_depth: Option<usize>) -> Vec<PathBuf> {
        let Some(&start) = self.node_index.get(from) else {
            return Vec::new();
        };

        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();
        queue.push_back((start, 0usize));
        visited.insert(start);

        let mut result = Vec::new();

        while let Some((node, depth)) = queue.pop_front() {
            if let Some(max) = max_depth {
                if depth >= max {
                    continue;
                }
            }
            for neighbor in self.inner.neighbors_directed(node, Direction::Outgoing) {
                if visited.insert(neighbor) {
                    result.push(self.inner[neighbor].clone());
                    queue.push_back((neighbor, depth + 1));
                }
            }
        }

        result.sort();
        result
    }

    /// All transitive dependents (reverse graph BFS)
    pub fn transitive_dependents(&self, target: &Path) -> Vec<PathBuf> {
        let Some(&start) = self.node_index.get(target) else {
            return Vec::new();
        };

        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();
        queue.push_back(start);
        visited.insert(start);

        let mut result = Vec::new();

        while let Some(node) = queue.pop_front() {
            for neighbor in self.inner.neighbors_directed(node, Direction::Incoming) {
                if visited.insert(neighbor) {
                    result.push(self.inner[neighbor].clone());
                    queue.push_back(neighbor);
                }
            }
        }

        result.sort();
        result
    }

    /// Shortest path between two nodes (BFS)
    pub fn shortest_path(&self, from: &Path, to: &Path) -> Option<Vec<PathBuf>> {
        let (&start, &end) = (self.node_index.get(from)?, self.node_index.get(to)?);

        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();
        let mut parent: HashMap<NodeIndex, NodeIndex> = HashMap::new();

        queue.push_back(start);
        visited.insert(start);

        while let Some(node) = queue.pop_front() {
            if node == end {
                // Reconstruct path
                let mut path = vec![self.inner[end].clone()];
                let mut current = end;
                while let Some(&prev) = parent.get(&current) {
                    path.push(self.inner[prev].clone());
                    current = prev;
                }
                path.reverse();
                return Some(path);
            }
            for neighbor in self.inner.neighbors_directed(node, Direction::Outgoing) {
                if visited.insert(neighbor) {
                    parent.insert(neighbor, node);
                    queue.push_back(neighbor);
                }
            }
        }

        None
    }

    /// Files sorted by number of dependents (descending)
    pub fn heaviest(&self) -> Vec<(PathBuf, usize)> {
        let mut counts: Vec<(PathBuf, usize)> = self
            .node_index
            .iter()
            .map(|(path, &idx)| {
                let count = self
                    .inner
                    .neighbors_directed(idx, Direction::Incoming)
                    .count();
                (path.clone(), count)
            })
            .collect();
        counts.sort_by(|a, b| b.1.cmp(&a.1));
        counts
    }

    /// In-degree count for a specific node
    pub fn in_degree(&self, path: &Path) -> usize {
        self.node_index
            .get(path)
            .map(|&idx| {
                self.inner
                    .neighbors_directed(idx, Direction::Incoming)
                    .count()
            })
            .unwrap_or(0)
    }

    /// Out-degree count for a specific node
    pub fn out_degree(&self, path: &Path) -> usize {
        self.node_index
            .get(path)
            .map(|&idx| {
                self.inner
                    .neighbors_directed(idx, Direction::Outgoing)
                    .count()
            })
            .unwrap_or(0)
    }

    pub fn edges_from(&self, path: &PathBuf) -> Vec<(PathBuf, ImportKind)> {
        if let Some(&idx) = self.node_index.get(path) {
            self.inner
                .neighbors_directed(idx, Direction::Outgoing)
                .map(|neighbor| {
                    let edge = self.inner.find_edge(idx, neighbor).unwrap();
                    (self.inner[neighbor].clone(), self.inner[edge].clone())
                })
                .collect()
        } else {
            Vec::new()
        }
    }
}
