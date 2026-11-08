const {test} = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const catalog = require('../assets/i18n/en.json');
const html = fs.readFileSync(path.join(__dirname, '../index.html'), 'utf8');
const startup = html.match(/<script id="startup-messages">([\s\S]*?)<\/script>/)[1];

function runStartup(document, fetch) {
    return vm.runInNewContext(startup, {document, fetch, Math: {random: () => 0, floor: Math.floor}});
}

test('the browser shell uses English catalog messages while loading', async () => {
    const loading = {};
    const attributes = {};
    const canvas = {setAttribute: (name, value) => { attributes[name] = value; }};
    const document = {
        getElementById: id => ({'ketchup-canvas': canvas, 'loading-label': loading})[id],
    };
    await runStartup(document, async url => {
        assert.equal(url, 'assets/i18n/en.json');
        return {ok: true, json: async () => catalog};
    });
    assert.equal(document.title, 'Untitled model - KetchUp3D');
    assert.equal(attributes['aria-label'], 'KetchUp3D modeling workspace');
    assert.equal(loading.textContent, 'Making room for your next big idea…');
});

test('a late catalog response leaves the open project title intact', async () => {
    const document = {
        title: 'Workshop - KetchUp3D',
        getElementById: id => id === 'ketchup-canvas' ? {setAttribute() {}} : null,
    };
    await runStartup(document, async () => ({ok: true, json: async () => catalog}));
    assert.equal(document.title, 'Workshop - KetchUp3D');
});
