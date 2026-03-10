export function normalize(data: unknown[]): unknown[] {
    return data.filter(Boolean);
}

export function log(message: string): void {
    console.log(`[LOG] ${message}`);
}

export function deprecatedHelper(): void {
    // This function is never called anywhere
}
