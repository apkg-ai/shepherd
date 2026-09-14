#!/usr/bin/env python3
"""Read-only handbook checks: references, step DAG, SQL baseline and fixture graph."""
import ast
import json
import re
import sqlite3
from pathlib import Path
from urllib.parse import unquote

ROOT = Path(__file__).resolve().parent


def load(path):
    return json.loads((ROOT / path).read_text())


def main():
    errors = []
    for path in ROOT.rglob('*.md'):
        for dest in re.findall(r'\[[^\]]*\]\(([^)]+)\)', path.read_text()):
            if dest.startswith(('http:', 'https:', '#', 'mailto:')):
                continue
            dest = unquote(dest.split('#')[0])
            if dest and not (path.parent / dest).exists():
                errors.append(f'Broken link: {path.relative_to(ROOT)} -> {dest}')
    steps = load('steps/manifest.json')
    by_id = {s['id']: s for s in steps}
    assert len(by_id) == len(steps)
    issue_map = load('steps/github-issues.json')
    assert set(issue_map['steps']) == {f'{i:03d}' for i in by_id}
    assert len(set(issue_map['steps'].values())) == len(by_id)
    visited, active = set(), set()

    def visit(i):
        assert i not in active, f'Step cycle at {i}'
        if i in visited:
            return
        active.add(i)
        step = by_id[i]
        text = (ROOT / 'steps' / step['file']).read_text()
        for heading in ('Objective and prerequisites', 'Files and boundaries',
                        'Ordered implementation', 'Acceptance tests', 'Verification',
                        'Completion checklist', 'Implementation handoff record'):
            assert heading in text, (i, heading)
        for dep in step['requires']:
            assert dep in by_id
            visit(dep)
        active.remove(i)
        visited.add(i)

    for i in by_id:
        visit(i)
    for file in ('contracts/openapi.yaml', 'contracts/asyncapi.yaml', 'contracts/export.schema.json'):
        doc = load(file)

        def walk(node):
            if isinstance(node, dict):
                if '$ref' in node:
                    target = doc
                    assert node['$ref'].startswith('#/'), node['$ref']
                    for part in node['$ref'][2:].split('/'):
                        target = target[part.replace('~1', '/').replace('~0', '~')]
                for val in node.values():
                    walk(val)
            elif isinstance(node, list):
                for val in node:
                    walk(val)
        walk(doc)
    api = load('contracts/openapi.yaml')
    actual = {o['operationId'] for item in api['paths'].values() for o in item.values()}
    assert actual == {o['operation_id'] for o in load('contracts/operations.json')}
    assert actual == {o['operation_id'] for o in load('contracts/adapter-map.json')}
    assert actual == {o['operation_id'] for o in load('examples/operations.json')}
    for route, item in api['paths'].items():
        for op in item.values():
            assert set(re.findall(r'{([^}]+)}', route)) == {
                p['name'] for p in op['parameters'] if p['in'] == 'path'}
    db = sqlite3.connect(':memory:')
    db.executescript((ROOT / 'contracts/schema.sql').read_text())
    assert db.execute('PRAGMA integrity_check').fetchone()[0] == 'ok'
    assert not db.execute('PRAGMA foreign_key_check').fetchall()
    fixture = load('examples/reference-project.json')
    groups = ('goals', 'epics', 'tasks', 'task_types', 'dependencies', 'documents',
              'document_revisions', 'sessions', 'submissions', 'reviews', 'actors')
    ids = [fixture['project']['id']] + [x['id'] for g in groups for x in fixture[g]]
    assert len(ids) == len(set(ids)), 'Duplicate fixture IDs'
    goals = {g['id']: g for g in fixture['goals']}
    epics = {e['id']: e for e in fixture['epics']}
    tasks = {t['id']: t for t in fixture['tasks']}
    for e in epics.values():
        assert e['goal_id'] in goals
    for t in tasks.values():
        assert t['epic_id'] in epics
    # Persist the real fixture: schema creation alone cannot reveal mismatched
    # composite foreign keys or required columns.
    db.execute('BEGIN')
    for table, records in [('actors', fixture['actors']), ('projects', [fixture['project']]),
                           ('goals', fixture['goals']), ('task_types', fixture['task_types']),
                           ('epics', fixture['epics']), ('tasks', fixture['tasks'])]:
        columns = {r[1] for r in db.execute(f'PRAGMA table_info({table})')}
        for record in records:
            row = {k: json.dumps(v) if isinstance(v, (dict, list)) else v
                   for k, v in record.items() if k in columns}
            if table == 'actors':
                row['created_at'] = fixture['exported_at']
            keys = ','.join(row)
            placeholders = ','.join('?' for _ in row)
            db.execute(f'INSERT INTO {table} ({keys}) VALUES ({placeholders})', tuple(row.values()))
    db.commit()
    for prefix in ('block', 'waiver', 'cancellation', 'archive'):
        try:
            db.execute(f'UPDATE tasks SET {prefix}_actor_id=?, {prefix}_created_at=? WHERE id=?',
                       (fixture['actors'][0]['id'], fixture['exported_at'], fixture['tasks'][0]['id']))
        except sqlite3.IntegrityError:
            db.rollback()
        else:
            raise AssertionError(f'Incomplete {prefix} record passed SQL CHECK')
    assert not db.execute('PRAGMA foreign_key_check').fetchall()
    for level, entities, owner in [('epic', epics, 'goal_id'), ('task', tasks, 'epic_id')]:
        edges = {}
        for dep in fixture['dependencies']:
            if dep['level'] != level:
                continue
            d, p = dep['dependent_id'], dep['prerequisite_id']
            assert d != p and entities[d][owner] == entities[p][owner]
            edges.setdefault(d, []).append(p)
        def acyclic(node, trail):
            assert node not in trail, f'Fixture cycle {node}'
            for predecessor in edges.get(node, []):
                acyclic(predecessor, trail | {node})
        for node in edges:
            acyclic(node, set())
    for script in ROOT.rglob('*.py'):
        ast.parse(script.read_text(), filename=str(script))
    if errors:
        raise SystemExit('\n'.join(errors))
    print(f'PASS: {len(steps)} step DAG; {len(actual)} operations; local links, schema refs, SQL baseline, fixture ownership/DAG, Python syntax.')


if __name__ == '__main__':
    main()
