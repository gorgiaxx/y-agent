/**
 * DagGraph -- pure SVG DAG renderer.
 *
 * Renders a DagVisualization (nodes + edges) as an interactive SVG graph.
 * Uses topological ordering for layered layout (simplified Sugiyama).
 * Color-coded by node type with smooth animations.
 */
import { useMemo } from 'react';
import type { DagVisualization, DagNode, DagEdge } from '../../hooks/useAutomation';

interface DagGraphProps {
  dag: DagVisualization;
  width?: number;
  height?: number;
}

// Node dimensions
const NODE_W = 160;
const NODE_H = 48;
const LAYER_GAP_X = 200;
const NODE_GAP_Y = 70;
const PADDING = 40;

/** Color palette by node type -- uses CSS custom properties for theme support. */
const NODE_COLORS: Record<string, { bg: string; border: string; text: string }> = {
  task: { bg: 'var(--surface-secondary)', border: 'var(--accent)', text: 'var(--text-primary)' },
  condition: { bg: 'var(--surface-secondary)', border: 'var(--warning)', text: 'var(--text-primary)' },
  parallel: { bg: 'var(--surface-secondary)', border: 'var(--success)', text: 'var(--text-primary)' },
  start: { bg: 'var(--surface-secondary)', border: 'var(--info)', text: 'var(--text-primary)' },
  end: { bg: 'var(--surface-secondary)', border: 'var(--error)', text: 'var(--text-primary)' },
  default: { bg: 'var(--surface-secondary)', border: 'var(--accent)', text: 'var(--text-primary)' },
};

/** Assign layers to nodes based on topological order and edge dependencies. */
function assignLayers(
  nodes: DagNode[],
  edges: DagEdge[],
  topoOrder: string[],
): Map<string, number> {
  const layers = new Map<string, number>();

  // Build incoming map
  const incoming = new Map<string, string[]>();
  for (const n of nodes) incoming.set(n.id, []);
  for (const e of edges) {
    const list = incoming.get(e.target) ?? [];
    list.push(e.source);
    incoming.set(e.target, list);
  }

  // Assign layers: layer = max(layer of predecessors) + 1
  for (const id of topoOrder) {
    const preds = incoming.get(id) ?? [];
    if (preds.length === 0) {
      layers.set(id, 0);
    } else {
      const maxPredLayer = Math.max(...preds.map((p) => layers.get(p) ?? 0));
      layers.set(id, maxPredLayer + 1);
    }
  }

  return layers;
}

/** Compute node positions from layers. */
function layoutNodes(
  nodes: DagNode[],
  edges: DagEdge[],
  topoOrder: string[],
): Map<string, { x: number; y: number }> {
  const layers = assignLayers(nodes, edges, topoOrder);
  const positions = new Map<string, { x: number; y: number }>();

  // Group by layer
  const layerGroups = new Map<number, string[]>();
  for (const [id, layer] of layers) {
    const group = layerGroups.get(layer) ?? [];
    group.push(id);
    layerGroups.set(layer, group);
  }

  // Position each layer
  for (const [layer, ids] of layerGroups) {
    const x = PADDING + layer * LAYER_GAP_X;
    const totalHeight = (ids.length - 1) * NODE_GAP_Y;
    const startY = PADDING + (300 - totalHeight) / 2; // Center vertically
    ids.forEach((id, idx) => {
      positions.set(id, { x, y: Math.max(PADDING, startY + idx * NODE_GAP_Y) });
    });
  }

  return positions;
}

/** Compute SVG path for a curved edge. */
function edgePath(
  sx: number,
  sy: number,
  tx: number,
  ty: number,
): string {
  const mx = (sx + tx) / 2;
  return `M ${sx} ${sy} C ${mx} ${sy}, ${mx} ${ty}, ${tx} ${ty}`;
}

export function DagGraph({ dag, width, height }: DagGraphProps) {
  const positions = useMemo(
    () => layoutNodes(dag.nodes, dag.edges, dag.topological_order),
    [dag],
  );

  // Compute SVG viewBox
  const computedBounds = useMemo(() => {
    let maxX = 0;
    let maxY = 0;
    for (const { x, y } of positions.values()) {
      maxX = Math.max(maxX, x + NODE_W + PADDING);
      maxY = Math.max(maxY, y + NODE_H + PADDING);
    }
    return { w: Math.max(maxX, 400), h: Math.max(maxY, 300) };
  }, [positions]);

  const svgW = width ?? computedBounds.w;
  const svgH = height ?? computedBounds.h;

  if (dag.nodes.length === 0) {
    return (
      <div className="dag-graph-empty">
        <p>No nodes in this workflow</p>
      </div>
    );
  }

  return (
    <svg
      className="dag-graph"
      width={svgW}
      height={svgH}
      viewBox={`0 0 ${computedBounds.w} ${computedBounds.h}`}
    >
      <defs>
        {/* Arrowhead marker */}
        <marker
          id="dag-arrow"
          viewBox="0 0 10 7"
          refX="10"
          refY="3.5"
          markerWidth="8"
          markerHeight="6"
          orient="auto-start-reverse"
        >
          <path d="M 0 0 L 10 3.5 L 0 7 z" fill="var(--text-muted)" />
        </marker>
        {/* Glow filter */}
        <filter id="dag-glow" x="-20%" y="-20%" width="140%" height="140%">
          <feGaussianBlur stdDeviation="3" result="blur" />
          <feMerge>
            <feMergeNode in="blur" />
            <feMergeNode in="SourceGraphic" />
          </feMerge>
        </filter>
      </defs>

      {/* Edges */}
      {dag.edges.map((edge, i) => {
        const from = positions.get(edge.source);
        const to = positions.get(edge.target);
        if (!from || !to) return null;

        const sx = from.x + NODE_W;
        const sy = from.y + NODE_H / 2;
        const tx = to.x;
        const ty = to.y + NODE_H / 2;

        return (
          <g key={`edge-${i}`}>
            <path
              d={edgePath(sx, sy, tx, ty)}
              fill="none"
              stroke="var(--border)"
              strokeWidth={2}
              markerEnd="url(#dag-arrow)"
              className="dag-edge"
            />
            {edge.label && (
              <text
                x={(sx + tx) / 2}
                y={(sy + ty) / 2 - 8}
                textAnchor="middle"
                fill="var(--text-secondary)"
                fontSize={11}
                className="dag-edge-label"
              >
                {edge.label}
              </text>
            )}
          </g>
        );
      })}

      {/* Nodes */}
      {dag.nodes.map((node) => {
        const pos = positions.get(node.id);
        if (!pos) return null;

        const colors = NODE_COLORS[node.task_type ?? node.node_type ?? 'default'] ?? NODE_COLORS.default;
        const displayLabel = node.name ?? node.label ?? node.id;
        const displayType = node.task_type ?? node.node_type ?? '';

        return (
          <g key={node.id} className="dag-node" filter="url(#dag-glow)">
            <rect
              x={pos.x}
              y={pos.y}
              width={NODE_W}
              height={NODE_H}
              rx={8}
              ry={8}
              fill={colors.bg}
              stroke={colors.border}
              strokeWidth={2}
            />
            {/* Node label */}
            <text
              x={pos.x + NODE_W / 2}
              y={pos.y + NODE_H / 2 - 4}
              textAnchor="middle"
              dominantBaseline="middle"
              fill={colors.text}
              fontSize={13}
              fontWeight={500}
              className="dag-node-label"
            >
              {displayLabel.length > 18
                ? displayLabel.slice(0, 16) + '..'
                : displayLabel}
            </text>
            {/* Type badge */}
            <text
              x={pos.x + NODE_W / 2}
              y={pos.y + NODE_H / 2 + 14}
              textAnchor="middle"
              dominantBaseline="middle"
              fill={colors.border}
              fontSize={10}
              fontWeight={400}
              opacity={0.8}
            >
              {displayType}
            </text>
          </g>
        );
      })}
    </svg>
  );
}
