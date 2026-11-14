const {test} = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const catalog = require('../assets/i18n/en_US.json');
const html = fs.readFileSync(path.join(__dirname, '../index.html'), 'utf8');
const startup = html.match(/<script id="startup-messages">([\s\S]*?)<\/script>/)[1];

function runStartup(document, fetch) {
    return vm.runInNewContext(startup, {document, fetch, Math: {random: () => 0, floor: Math.floor}});
}

// Catalog text must not introduce destinations absent from the source message.
// Include bare domains and normalize full-width ASCII used to disguise addresses.
function assertSafeTranslation(source, translated, key) {
    assert.doesNotMatch(translated, /[\p{Cc}\p{Cf}]/gu, key);
    const references = text => text.normalize('NFKC').match(
        /\b[a-z][a-z\d+.-]*:\/\/[^\s<>"'，。；（）]+|\b(?:javascript|data|mailto):[^\s<>"'，。；（）]*|\b(?:[a-z\d](?:[a-z\d-]*[a-z\d])?\.)+[a-z]{2,63}\b/gi,
    ) || [];
    const allowed = new Set(references(source));
    for (const reference of references(translated)) {
        assert.ok(allowed.has(reference), `${key}: introduced address ${reference}`);
    }
}

test('translation safety rejects introduced destinations and hidden text controls', () => {
    for (const text of ['https://example.com/login', 'example.cn', 'ｗｗｗ．example．com',
        'javascript:alert(1)', 'data:text/html,test', 'Click\u202ehere', 'exam\u200bple.com']) {
        assert.throws(() => assertSafeTranslation('Open project', text, 'fixture'));
    }
    assertSafeTranslation('Visit https://example.com/help', '访问 https://example.com/help', 'fixture');
    assertSafeTranslation('Save .bup', '保存 .bup', 'fixture');
});

test('the browser shell uses English catalog messages while loading', async () => {
    const loading = {};
    const attributes = {};
    const canvas = {setAttribute: (name, value) => { attributes[name] = value; }};
    const document = {
        getElementById: id => ({'benchup-canvas': canvas, 'loading-label': loading})[id],
    };
    await runStartup(document, async url => {
        assert.equal(url, 'assets/i18n/en_US.json');
        return {ok: true, json: async () => catalog};
    });
    assert.equal(document.title, 'Untitled model - BenchUp');
    assert.equal(attributes['aria-label'], 'BenchUp modeling workspace');
    assert.equal(loading.textContent, 'Making room for your next big idea…');
});

test('a late catalog response leaves the open project title intact', async () => {
    const document = {
        title: 'Workshop - BenchUp',
        getElementById: id => id === 'benchup-canvas' ? {setAttribute() {}} : null,
    };
    await runStartup(document, async () => ({ok: true, json: async () => catalog}));
    assert.equal(document.title, 'Workshop - BenchUp');
});

for (const [language, file] of [['Chinese', 'zh_CN'], ['Brazilian Portuguese', 'pt_BR']]) {
    test(`${language} messages preserve English catalog keys and format arguments`, () => {
        const translated = require(`../assets/i18n/${file}.json`);
        assert.deepEqual(Object.keys(translated).sort(), Object.keys(catalog).sort());
        const argumentsOf = text => [...text.matchAll(/(?<!\{)\{([^{}]*)\}(?!\})/g)]
            .map(match => match[1]).sort();
        for (const [key, message] of Object.entries(catalog)) {
            assert.equal(typeof translated[key], typeof message, key);
            if (typeof message === 'string') {
                assert.deepEqual(argumentsOf(translated[key]), argumentsOf(message), key);
                assertSafeTranslation(message.replace(/[\n\r\t]/g, ''), translated[key].replace(/[\n\r\t]/g, ''), key);
            } else {
                assert.deepEqual(Object.keys(translated[key]).sort(), Object.keys(message).sort(), key);
                for (const [form, text] of Object.entries(message)) {
                    assert.deepEqual(argumentsOf(translated[key][form]), argumentsOf(text), `${key}.${form}`);
                    assertSafeTranslation(text.replace(/[\n\r\t]/g, ''), translated[key][form].replace(/[\n\r\t]/g, ''), `${key}.${form}`);
                }
            }
        }
    });
}

test('Chinese tool and material controls contain translated text', () => {
    const translated = require('../assets/i18n/zh_CN.json');
    // Catalog shape and placeholder checks alone also accept English copied
    // into another language. Guard the feature families that had such gaps.
    const families = /^(?:router\.|wood_glue\.|finish_settings\.|time_estimates\.router|orbit\.|space_mouse\.|scale_tool\.|core\.reference_images\.|app\.arc|app\.draw_arc$|app\.snap_only_visible|tools\.arc|measurement_tools\.(?:add_angle|adjust_angle|undefined_angle)|outliner\.angle_dimension$)/;
    for (const [key, message] of Object.entries(catalog)) {
        if (families.test(key) && typeof message === 'string' && /[A-Za-z]{3}/.test(message)) {
            assert.notEqual(translated[key], message, `${key}: untranslated English`);
        }
    }
});

test('runtime translations are formatted before being passed to Rust output macros', () => {
    const rustFiles = directory => fs.readdirSync(directory, {withFileTypes: true}).flatMap(entry => {
        const file = path.join(directory, entry.name);
        return entry.isDirectory() ? rustFiles(file) : entry.name.endsWith('.rs') ? [file] : [];
    });
    for (const file of rustFiles(path.join(__dirname, '../crates'))) {
        const source = fs.readFileSync(file, 'utf8');
        for (const [, key] of source.matchAll(/\btr!\(\s*"([^"]+)"/g)) {
            assert.ok(Object.hasOwn(catalog, key), `${file}: unknown message ${key}`);
        }
        // tr! now expands to a runtime expression; format-string positions require literals.
        assert.doesNotMatch(source,
            /\b(?:format|format_args|print|println|eprint|eprintln|panic)!\(\s*(?:benchup_core::)?tr!\(/,
            file);
        assert.doesNotMatch(source,
            /\b(?:write|writeln)!\(\s*[^,]+,\s*(?:benchup_core::)?tr!\(/,
            file);
    }
});
