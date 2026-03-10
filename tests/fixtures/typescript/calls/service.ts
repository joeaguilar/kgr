import { query } from './db';

export class UserService {
    getUser(userId: number): Record<string, unknown> {
        return query("SELECT * FROM users WHERE id = ?", userId);
    }

    listUsers(): Record<string, unknown>[] {
        return query("SELECT * FROM users");
    }
}

export function fetchUsers(): Record<string, unknown>[] {
    const svc = new UserService();
    return svc.listUsers();
}
