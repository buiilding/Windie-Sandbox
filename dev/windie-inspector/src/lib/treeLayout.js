/**
 * Computes a balanced top-down layout for the projected conversation tree.
 *
 * Each node reserves horizontal space for its complete subtree. Children are
 * placed next to one another inside that space, which keeps a parent centered
 * over its descendants and allows sibling branches to extend in both
 * directions.
 */
export function layoutTree(tree) {
  const nodes = tree?.nodes || {};
  const rootIds = tree?.rootIds || [];
  const NODE_W = 180;
  const NODE_H = 78;
  const GROUP_H = 30;
  const H_GAP = 40;
  const V_GAP = 28;
  const PAD_X = 40;
  const PAD_Y = 40;

  if (!rootIds.length) {
    return { positions: {}, edges: [], width: 900, height: 280, NODE_W, NODE_H };
  }

  const subtreeWidths = new Map();
  const depths = new Map();
  const rowHeights = new Map();
  const measuring = new Set();

  const measure = (id, depth) => {
    if (subtreeWidths.has(id)) return subtreeWidths.get(id);
    const node = nodes[id];
    if (!node || measuring.has(id)) return NODE_W;

    measuring.add(id);
    depths.set(id, depth);
    const nodeHeight = node.kind === "execution_group" ? GROUP_H : NODE_H;
    rowHeights.set(depth, Math.max(rowHeights.get(depth) || 0, nodeHeight));

    const childWidths = (node.childrenIds || [])
      .filter((childId) => nodes[childId])
      .map((childId) => measure(childId, depth + 1));
    const childrenWidth = childWidths.length
      ? childWidths.reduce((total, width) => total + width, 0) + H_GAP * (childWidths.length - 1)
      : 0;
    const width = Math.max(NODE_W, childrenWidth);

    measuring.delete(id);
    subtreeWidths.set(id, width);
    return width;
  };

  const rootWidths = rootIds.filter((id) => nodes[id]).map((id) => measure(id, 0));
  const rowY = new Map();
  let y = PAD_Y;
  [...rowHeights.keys()].sort((a, b) => a - b).forEach((depth) => {
    rowY.set(depth, y);
    y += rowHeights.get(depth) + V_GAP;
  });

  const positions = {};
  const place = (id, left) => {
    const node = nodes[id];
    if (!node || positions[id]) return;

    const depth = depths.get(id) || 0;
    const subtreeWidth = subtreeWidths.get(id) || NODE_W;
    const height = node.kind === "execution_group" ? GROUP_H : NODE_H;
    positions[id] = {
      x: left + (subtreeWidth - NODE_W) / 2,
      y: rowY.get(depth) || PAD_Y,
      depth,
      height,
    };

    let childLeft = left;
    (node.childrenIds || []).forEach((childId) => {
      if (!nodes[childId]) return;
      place(childId, childLeft);
      childLeft += (subtreeWidths.get(childId) || NODE_W) + H_GAP;
    });
  };

  let rootLeft = PAD_X;
  rootIds.filter((id) => nodes[id]).forEach((rootId, index) => {
    place(rootId, rootLeft);
    rootLeft += rootWidths[index] + H_GAP;
  });

  const edges = [];
  Object.values(nodes).forEach((node) => {
    (node.childrenIds || []).forEach((childId) => {
      if (nodes[childId]) edges.push({ from: node.id, to: childId });
    });
  });

  const width = Math.max(900, rootLeft - H_GAP + PAD_X);
  const height = Math.max(...Object.values(positions).map((position) => position.y + position.height), 0) + PAD_Y;
  return { positions, edges, width, height, NODE_W, NODE_H };
}
