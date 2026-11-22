import { getJson, postJson } from './api.js';

// Who is signed in: undefined until asked, then the user object from /api/session, or null.
let user;
const listeners = new Set();

function set(next) {
	user = next;
	for (const fn of listeners) fn(user);
}

/** The signed-in user, or null. Asks the server the first time. */
export async function getUser() {
	if (user === undefined) await refreshUser();
	return user;
}

/** Asks the server again (after a run, the remaining count changes). */
export async function refreshUser() {
	try {
		set((await getJson('/api/session')).user);
	} catch (error) {
		if (error?.name !== 'ApiError') throw error;
		// The server is unreachable or broken: treat as signed out; pages will report the failure themselves.
		set(null);
	}
	return user;
}

/** Records the user returned by a log-in or profile change. */
export const setUser = (next) => set(next);

export async function signOut() {
	await postJson('/api/auth/sign-out');
	set(null);
}

/** Calls `fn(user)` whenever the signed-in user changes. */
export function onUserChange(fn) {
	listeners.add(fn);
}

/** Where to send someone who must log in first, coming back to `path` afterwards. */
export const logInUrl = (path = location.pathname + location.search) => `/log-in?next=${encodeURIComponent(path)}`;

/** Where to send someone who has no account yet, going on to `path` once they have one. */
export const signUpUrl = (path = location.pathname + location.search) => `/sign-up?next=${encodeURIComponent(path)}`;
