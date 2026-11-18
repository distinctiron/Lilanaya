/** An error answered by the API: `code` and `message`, plus `details` ([{ path, message }]) when there are any. */
export class ApiError extends Error {
	constructor(status, code, message, details = []) {
		super(message);
		this.name = 'ApiError';
		this.status = status;
		this.code = code;
		this.details = details;
	}
}

async function request(method, path, body) {
	let res;
	try {
		res = await fetch(path, {
			method,
			headers: { Accept: 'application/json', ...(body === undefined ? {} : { 'Content-Type': 'application/json' }) },
			body: body === undefined ? undefined : JSON.stringify(body)
		});
	} catch {
		throw new ApiError(0, 'network', 'The server could not be reached.');
	}
	let data = null;
	try {
		data = await res.json();
	} catch {
		// Not JSON (for example a proxy error page); handled below.
	}
	if (!res.ok) {
		const e = data?.error;
		throw new ApiError(res.status, e?.code ?? 'http_error', e?.message ?? `The server answered ${res.status}.`, e?.details ?? []);
	}
	return data;
}

/** GET a JSON resource; rejects with ApiError on failure. */
export const getJson = (path) => request('GET', path);

/** POST a JSON body; rejects with ApiError on failure. */
export const postJson = (path, body) => request('POST', path, body);

/** PUT a JSON body; rejects with ApiError on failure. */
export const putJson = (path, body) => request('PUT', path, body);

/** PATCH a JSON body; rejects with ApiError on failure. */
export const patchJson = (path, body) => request('PATCH', path, body);

/** DELETE a resource; rejects with ApiError on failure. */
export const deleteJson = (path) => request('DELETE', path);
