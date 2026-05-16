import { ChevronRight } from "lucide-react";
import { useState } from "react";
import type { TreeNode } from "../lib/types";

export function TreeView({
  nodes,
  onSelect
}: {
  nodes: TreeNode[];
  onSelect: (node: TreeNode) => void;
}) {
  return (
    <div className="tree-view">
      {nodes.map((node) => (
        <TreeItem key={node.id} node={node} onSelect={onSelect} level={0} />
      ))}
    </div>
  );
}

function TreeItem({
  node,
  onSelect,
  level
}: {
  node: TreeNode;
  onSelect: (node: TreeNode) => void;
  level: number;
}) {
  const [open, setOpen] = useState(level < 1);
  const hasChildren = Boolean(node.children?.length);

  return (
    <div>
      <button
        className={`tree-row ${node.type}`}
        type="button"
        style={{ paddingLeft: `${8 + level * 16}px` }}
        onClick={() => {
          if (hasChildren) {
            setOpen((value) => !value);
          } else {
            onSelect(node);
          }
        }}
        onDoubleClick={() => onSelect(node)}
      >
        <ChevronRight className={open ? "open" : ""} size={14} aria-hidden="true" />
        <span>{node.label}</span>
      </button>
      {open && hasChildren
        ? node.children!.map((child) => (
            <TreeItem key={child.id} node={child} onSelect={onSelect} level={level + 1} />
          ))
        : null}
    </div>
  );
}
