// SPDX-License-Identifier: MIT OR Apache-2.0
//
// A minimal directed graph with breadth-first traversal. Distinct "adjacency"/
// "traversal"/"visited" vocabulary so a graph-search query resolves here.

package corpus

// Graph is a directed graph stored as an adjacency list keyed by node id.
type Graph struct {
	adjacency map[int][]int
}

// NewGraph returns an empty directed graph ready for edges.
func NewGraph() *Graph {
	return &Graph{adjacency: make(map[int][]int)}
}

// AddEdge records a directed edge from `from` to `to`.
func (g *Graph) AddEdge(from, to int) {
	g.adjacency[from] = append(g.adjacency[from], to)
}

// BreadthFirstOrder returns the node ids reachable from `start` in
// breadth-first order, visiting each node at most once.
func (g *Graph) BreadthFirstOrder(start int) []int {
	visited := make(map[int]bool)
	queue := []int{start}
	order := []int{}
	for len(queue) > 0 {
		node := queue[0]
		queue = queue[1:]
		if visited[node] {
			continue
		}
		visited[node] = true
		order = append(order, node)
		queue = append(queue, g.adjacency[node]...)
	}
	return order
}

// CountReachable returns how many distinct nodes are reachable from `start`,
// including the start node itself.
func (g *Graph) CountReachable(start int) int {
	return len(g.BreadthFirstOrder(start))
}
