use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct DirectedEdge {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VertexStatus {
    pub vertex: String,
    pub graph_maximal: bool,
    pub supported_outgoing_win: bool,
    pub merely_undefeated: bool,
    pub member_of_supported_cycle: bool,
    pub has_supported_loss: bool,
    pub affected_by_unresolved_comparison: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphSummary {
    pub vertices: Vec<String>,
    pub edges: Vec<DirectedEdge>,
    pub strongly_connected_components: Vec<Vec<String>>,
    pub condensation_edges: Vec<(usize, usize)>,
    pub source_component_indices: Vec<usize>,
    pub maximal_vertices: Vec<String>,
    pub unresolved_directions: Vec<DirectedEdge>,
    pub vertex_status: Vec<VertexStatus>,
}

pub fn analyze_graph(
    vertices: impl IntoIterator<Item = String>,
    edges: impl IntoIterator<Item = DirectedEdge>,
    unresolved_directions: impl IntoIterator<Item = DirectedEdge>,
) -> Result<GraphSummary> {
    let vertices: BTreeSet<String> = vertices.into_iter().collect();
    if vertices.is_empty() {
        return Err(Error::InvalidReduction("graph has no vertices".into()));
    }
    let edges: BTreeSet<DirectedEdge> = edges.into_iter().collect();
    let unresolved: BTreeSet<DirectedEdge> = unresolved_directions.into_iter().collect();
    for edge in edges.iter().chain(&unresolved) {
        if edge.from == edge.to || !vertices.contains(&edge.from) || !vertices.contains(&edge.to) {
            return Err(Error::InvalidReduction(format!(
                "invalid directed graph edge {} -> {}",
                edge.from, edge.to
            )));
        }
    }
    let adjacency = adjacency_map(&vertices, &edges, false);
    let reverse = adjacency_map(&vertices, &edges, true);
    let mut visited = BTreeSet::new();
    let mut finish_order = Vec::new();
    for vertex in &vertices {
        dfs_finish(vertex, &adjacency, &mut visited, &mut finish_order);
    }
    visited.clear();
    let mut components = Vec::new();
    for vertex in finish_order.iter().rev() {
        if visited.contains(vertex) {
            continue;
        }
        let mut component = Vec::new();
        dfs_collect(vertex, &reverse, &mut visited, &mut component);
        component.sort();
        components.push(component);
    }
    components.sort_by(|left, right| left[0].cmp(&right[0]));
    let mut component_of = BTreeMap::new();
    for (index, component) in components.iter().enumerate() {
        for vertex in component {
            component_of.insert(vertex.clone(), index);
        }
    }
    let mut condensation = BTreeSet::new();
    let mut incoming_components = vec![0_usize; components.len()];
    for edge in &edges {
        let from = component_of[&edge.from];
        let to = component_of[&edge.to];
        if from != to && condensation.insert((from, to)) {
            incoming_components[to] += 1;
        }
    }
    let source_component_indices: Vec<_> = incoming_components
        .iter()
        .enumerate()
        .filter_map(|(index, &incoming)| (incoming == 0).then_some(index))
        .collect();
    let maximal: BTreeSet<_> = source_component_indices
        .iter()
        .flat_map(|&index| components[index].iter().cloned())
        .collect();
    let unresolved_vertices: BTreeSet<_> = unresolved
        .iter()
        .flat_map(|edge| [edge.from.clone(), edge.to.clone()])
        .collect();
    let mut vertex_status = Vec::new();
    for vertex in &vertices {
        let outgoing = edges.iter().any(|edge| edge.from == *vertex);
        let incoming = edges.iter().any(|edge| edge.to == *vertex);
        let component = component_of[vertex];
        let in_cycle = components[component].len() > 1;
        vertex_status.push(VertexStatus {
            vertex: vertex.clone(),
            graph_maximal: maximal.contains(vertex),
            supported_outgoing_win: outgoing,
            merely_undefeated: maximal.contains(vertex) && !outgoing && !incoming,
            member_of_supported_cycle: in_cycle,
            has_supported_loss: incoming,
            affected_by_unresolved_comparison: unresolved_vertices.contains(vertex),
        });
    }
    Ok(GraphSummary {
        vertices: vertices.into_iter().collect(),
        edges: edges.into_iter().collect(),
        strongly_connected_components: components,
        condensation_edges: condensation.into_iter().collect(),
        source_component_indices,
        maximal_vertices: maximal.into_iter().collect(),
        unresolved_directions: unresolved.into_iter().collect(),
        vertex_status,
    })
}

fn adjacency_map(
    vertices: &BTreeSet<String>,
    edges: &BTreeSet<DirectedEdge>,
    reverse: bool,
) -> BTreeMap<String, Vec<String>> {
    let mut adjacency: BTreeMap<_, Vec<_>> = vertices
        .iter()
        .map(|vertex| (vertex.clone(), Vec::new()))
        .collect();
    for edge in edges {
        let (from, to) = if reverse {
            (&edge.to, &edge.from)
        } else {
            (&edge.from, &edge.to)
        };
        adjacency
            .get_mut(from)
            .expect("vertex exists")
            .push(to.clone());
    }
    for neighbors in adjacency.values_mut() {
        neighbors.sort();
    }
    adjacency
}

fn dfs_finish(
    vertex: &str,
    adjacency: &BTreeMap<String, Vec<String>>,
    visited: &mut BTreeSet<String>,
    finish_order: &mut Vec<String>,
) {
    if !visited.insert(vertex.to_owned()) {
        return;
    }
    for neighbor in &adjacency[vertex] {
        dfs_finish(neighbor, adjacency, visited, finish_order);
    }
    finish_order.push(vertex.to_owned());
}

fn dfs_collect(
    vertex: &str,
    adjacency: &BTreeMap<String, Vec<String>>,
    visited: &mut BTreeSet<String>,
    component: &mut Vec<String>,
) {
    if !visited.insert(vertex.to_owned()) {
        return;
    }
    component.push(vertex.to_owned());
    for neighbor in &adjacency[vertex] {
        dfs_collect(neighbor, adjacency, visited, component);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_scc_preserves_cycle_and_excludes_dominated_component() {
        let graph = analyze_graph(
            ["a", "b", "c"].into_iter().map(str::to_owned),
            [
                DirectedEdge {
                    from: "a".into(),
                    to: "b".into(),
                },
                DirectedEdge {
                    from: "b".into(),
                    to: "a".into(),
                },
                DirectedEdge {
                    from: "a".into(),
                    to: "c".into(),
                },
            ],
            [],
        )
        .unwrap();
        assert_eq!(graph.maximal_vertices, vec!["a", "b"]);
        assert_eq!(graph.source_component_indices.len(), 1);
    }

    #[test]
    fn no_edges_retains_every_vertex_as_merely_undefeated() {
        let graph = analyze_graph(["a", "b"].into_iter().map(str::to_owned), [], []).unwrap();
        assert_eq!(graph.maximal_vertices, vec!["a", "b"]);
        assert!(
            graph
                .vertex_status
                .iter()
                .all(|status| status.merely_undefeated)
        );
    }
}
