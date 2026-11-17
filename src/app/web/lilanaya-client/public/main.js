import { getJson } from './api.js';

const status = document.getElementById('status');

try {
	const body = await getJson('/api/hello');
	status.textContent = body.message;
} catch (e) {
	status.textContent = `API unreachable: ${e.message}`;
}
