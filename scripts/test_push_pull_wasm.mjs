#!/usr/bin/env node
// Exercise the running development build's actual Rust geometry in a worker
// environment. This script never starts a build or touches a user's project.
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {resolve, dirname} from 'node:path';
import {fileURLToPath, pathToFileURL} from 'node:url';

const dist = resolve(process.argv[2] ?? resolve(dirname(fileURLToPath(import.meta.url)), '../dist'));
const html = await readFile(resolve(dist, 'index.html'), 'utf8');
const js = html.match(/import init, \* as bindings from '\.\/(benchup-app-[^']+\.js)'/)?.[1];
const wasm = html.match(/module_or_path: '\.\/(benchup-app-[^']+\.wasm)'/)?.[1];
assert(js && wasm, 'An existing Trunk development output is required.');
const module = await import(pathToFileURL(resolve(dist, js)));
assert.equal(typeof module.benchup_validate_push_pull, 'function', 'The development output needs the push/pull validation export.');
await module.default({module_or_path: await readFile(resolve(dist, wasm))});

function box() {
    const model = {vertices: {}, edges: {}, faces: {}, next_id: 0};
    const positions = [[0,0,0], [5,0,0], [5,2,0], [0,2,0], [0,0,4], [5,0,4], [5,2,4], [0,2,4]];
    const vertices = positions.map(position => {
        const id = ++model.next_id;
        model.vertices[id] = {id, position};
        return id;
    });
    const edges = new Set();
    for (const indices of [[0,1,2,3], [4,5,6,7], [0,1,5,4], [1,2,6,5], [2,3,7,6], [3,0,4,7]]) {
        const ring = indices.map(i => vertices[i]);
        for (let i = 0; i < ring.length; i++) {
            const a = ring[i], b = ring[(i + 1) % ring.length];
            const key = [a,b].sort((a,b) => a-b).join(',');
            if (!edges.has(key)) {
                const id = ++model.next_id;
                model.edges[id] = {id, a, b};
                edges.add(key);
            }
        }
        const id = ++model.next_id;
        model.faces[id] = {id, vertices: ring, holes: []};
    }
    return {version: 2, name: 'Branched Ctrl extrusion regression', camera: {}, model};
}

function contains(model, face, point) {
    const coordinates = face.vertices.map(id => model.vertices[id].position);
    const normal = [0,1,2].find(axis => coordinates.every(p => Math.abs(p[axis] - point[axis]) < 1e-7));
    if (normal === undefined) return false;
    const axes = [0,1,2].filter(axis => axis !== normal);
    function inside(ring) {
        let contained = false;
        for (let i = 0, j = ring.length - 1; i < ring.length; j = i++) {
            const a = model.vertices[ring[i]].position, b = model.vertices[ring[j]].position;
            const [x,y] = axes;
            if ((a[y] > point[y]) !== (b[y] > point[y]) &&
                point[x] < (b[x] - a[x]) * (point[y] - a[y]) / (b[y] - a[y]) + a[x]) contained = !contained;
        }
        return contained;
    }
    return inside(face.vertices) && !face.holes.some(inside);
}

function push(doc, point, delta) {
    const face = Object.values(doc.model.faces).reverse().find(face => contains(doc.model, face, point));
    assert(face, `No face at ${point}`);
    return JSON.parse(module.benchup_validate_push_pull(JSON.stringify(doc), JSON.stringify({face: face.id, delta, new_faces: true})));
}

let doc = box();
for (const [point, delta] of [
    [[2.5,1,4], [0,0,1]],
    [[2.5,1,5], [0,0,1.5]],
    [[2.5,0,4.5], [0,-3,0]],
    [[5,1,4.5], [2,0,0]],
    [[7,1,4.5], [0.5,0,0]],
    [[7.25,0,4.5], [0,-3,0]],
]) doc = push(doc, point, delta).document;

const uses = new Map();
for (const face of Object.values(doc.model.faces)) {
    for (let i = 0; i < face.vertices.length; i++) {
        const key = [face.vertices[i], face.vertices[(i+1) % face.vertices.length]].sort((a,b) => a-b).join(',');
        uses.set(key, (uses.get(key) ?? 0) + 1);
    }
}
assert([...uses.values()].some(n => n >= 4), 'The reproduction must include branched four-face seams.');
for (const depth of [0.5, 3, 4]) {
    const result = push(doc, [7.25,-3,4.5], [0,depth,0]);
    assert.equal(typeof result.after_solid.Ok, 'number', JSON.stringify(result.after_solid));
    assert(Math.abs(result.after_solid.Ok - (86.5 - depth * 0.5)) < 1e-6);
    const point = [7.25, -3 + depth * 0.5, 5];
    assert(Object.values(result.document.model.faces).every(face => !contains(result.document.model, face, point)),
        'The removed top strip must have no original face or reversed overlapping wall.');
    console.log(`PASS: inward Ctrl push ${depth}: closed body, correct volume, no overlapping top strip`);
}
