// Naming the machines usage rows mention. Names come from the Machines registry, which only lists
// machines while the Machines consent is on; without it a machine is shown by its id's first block.
import type { HaryxMachine } from "./types";

export interface MachineLabel {
  id: string;
  name: string;
  signedIn: boolean | null;
  isThisMachine: boolean;
}

export function labelMachines(ids: readonly string[], registry: readonly HaryxMachine[]): MachineLabel[] {
  const byId = new Map(registry.map((m) => [m.machineId, m]));
  return ids.map((id) => {
    const m = byId.get(id);
    return m
      ? { id, name: m.name, signedIn: m.signedIn, isThisMachine: m.isThisMachine }
      : { id, name: `Machine ${id.split("-")[0]}`, signedIn: null, isThisMachine: false };
  });
}
