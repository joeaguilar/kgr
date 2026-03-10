export function query(sql: string, ...args: unknown[]): any {
    return [{ id: 1, name: "alice" }];
}

export function connect(): void {}

function internalReset(): void {
    // Private helper, never called outside this module
}
