import { ApiError } from './api.js';
import { button, linkButton } from './components/button.js';
import { renderHeader } from './components/header.js';
import { sectionHeading } from './components/section-heading.js';
import { h } from './dom.js';
import { howItWorksPage } from './pages/how-it-works.js';
import { landingPage } from './pages/landing.js';
import { profilePage } from './pages/profile.js';
import { resultsPage } from './pages/results.js';
import { runFormPage } from './pages/run-form.js';
import { runsPage } from './pages/runs.js';
import { confirmSignInPage, signInPage } from './pages/sign-in.js';
import { strategiesPage } from './pages/strategies.js';
import { chooseModePage, strategyEditorPage } from './pages/strategy-editor.js';
import { worldPage } from './pages/world.js';
import { worldsPage } from './pages/worlds.js';
import { startRouter } from './router.js';
import { getUser, onUserChange } from './session.js';

const header = document.getElementById('header');
const main = document.getElementById('main');

const notFound = async () => ({
	title: 'Not found',
	node: h('section', { class: 'stack' }, [
		sectionHeading('Game over: page not found', { level: 1 }),
		h('p', null, 'There is nothing at this address.'),
		h('p', null, linkButton('Back to worlds', '/worlds'))
	])
});

function errorPage(error, retry) {
	const missing = error instanceof ApiError && error.status === 404;
	return {
		title: missing ? 'Not found' : 'Error',
		node: h('section', { class: 'stack', role: 'alert' }, [
			sectionHeading(missing ? 'Not found' : 'Something went wrong', { level: 1 }),
			h('p', null, error instanceof ApiError ? error.message : 'The page could not be shown.'),
			h('p', { class: 'row' }, [missing ? null : button('Try again', { onclick: retry }), linkButton('Back to worlds', '/worlds')])
		])
	};
}

// The header shows who is signed in; draw it again whenever that changes.
let user;
const drawHeader = () => renderHeader(header, location.pathname, user);
onUserChange((next) => {
	user = next;
	drawHeader();
});
getUser().catch((error) => console.error(error));

startRouter(
	[
		['/', landingPage],
		['/how-it-works', howItWorksPage],
		['/worlds', worldsPage],
		['/worlds/:id', worldPage],
		['/strategies', strategiesPage],
		['/strategies/new', chooseModePage],
		['/strategies/new/:mode', strategyEditorPage],
		['/strategies/:id/edit', strategyEditorPage],
		['/sign-in', signInPage],
		['/sign-in/confirm', confirmSignInPage],
		['/profile', profilePage],
		['/runs', runsPage],
		['/runs/new', runFormPage],
		['/runs/:id', resultsPage]
	],
	{ main, onNavigate: drawHeader, notFound, errorPage }
);
