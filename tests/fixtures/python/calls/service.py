from . import db


class UserService:
    def get_user(self, user_id):
        return db.query("SELECT * FROM users WHERE id = ?", user_id)

    def list_users(self):
        return db.query("SELECT * FROM users")


def fetch_users():
    svc = UserService()
    return svc.list_users()
