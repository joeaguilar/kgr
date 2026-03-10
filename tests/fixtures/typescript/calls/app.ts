import { fetchUsers } from './service';
import { normalize, log } from './utils';

export function main(): string[] {
    const data = fetchUsers();
    const cleaned = normalize(data);
    log("processed users");
    return cleaned;
}

function cli(): void {
    const result = main();
    console.log(result);
}

cli();
