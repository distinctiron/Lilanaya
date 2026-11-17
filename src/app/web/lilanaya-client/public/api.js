/** GET a JSON resource from the backend; rejects on non-2xx responses. */
export async function getJson(path) {
	const res = await fetch(path, { headers: { Accept: 'application/json' } });
	if (!res.ok) throw new Error(`HTTP ${res.status}`);
	return res.json();
}
