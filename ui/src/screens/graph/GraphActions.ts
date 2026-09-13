import { createContext } from "react";

/** Context for actions and lens info that custom nodes need from the graph. */
export const GraphActionsContext = createContext<{
  toggleExpand: (taskId: string) => void;
  lens: "tree" | "flow";
}>({ toggleExpand: () => {}, lens: "flow" });
