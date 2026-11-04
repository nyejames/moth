/** @moth.sig branch_only || -> */
export function branchOnly() {
    throw new Error("statically false branch executed");
}
