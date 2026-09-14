#!/usr/bin/env python3
"""Generate/check planning contracts in a temporary directory, never application source.

Requires installed openapi-to-rust, Cargo cached dependencies, and ui/node_modules.
Use --node-dir only to select a locally installed Node executable explicitly.
"""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--node-dir', type=Path)
    args = parser.parse_args()
    plan = Path(__file__).resolve().parent
    repo = plan.parent
    scratch = Path(tempfile.mkdtemp(prefix='shepherd-plan-generation-'))
    env = dict(os.environ)
    if args.node_dir:
        env['PATH'] = str(args.node_dir) + os.pathsep + env.get('PATH', '')
    node = shutil.which('node', path=env.get('PATH'))
    if not node:
        parser.error('Activate Node from .nvmrc or pass --node-dir.')

    def run(*cmd):
        print('+ ' + ' '.join(map(str, cmd)), flush=True)
        subprocess.run(list(map(str, cmd)), cwd=scratch, env=env, check=True)

    print('Scratch artifacts:', scratch, flush=True)
    run(node, '--version')
    spec = plan / 'contracts/openapi.yaml'
    models = scratch / 'models'
    run('openapi-to-rust', 'generate', spec, '--types-only', '--output-dir',
        models / 'src/generated', '--json')
    fragment = (models / 'src/generated/REQUIRED_DEPS.toml').read_text()
    (models / 'Cargo.toml').write_text(
        '[package]\nname="shepherd-plan-model-check"\nversion="0.0.0"\nedition="2024"\n' + fragment)
    (models / 'src/lib.rs').write_text('pub mod generated;\n')
    run('cargo', 'check', '--offline', '--manifest-path', models / 'Cargo.toml')

    operations = json.loads((plan / 'contracts/operations.json').read_text())
    server = scratch / 'server'
    config = scratch / 'server.toml'
    config.write_text(
        '[generator]\nspec_path=' + json.dumps(str(spec)) + '\noutput_dir=' +
        json.dumps(str(server / 'src/generated')) + '\nmodule_name="shepherd"\n'
        '[features]\nenable_async_client=false\n'
        '[generator.types]\ndate_time="chrono"\nuuid="uuid"\n'
        '[server]\nframework="axum"\noperations=' + json.dumps([
            o['operation_id'] for o in operations
            if o['operation_id'] not in ('getEvents', 'exportProject', 'importProject')]) +
        '\nprune_models=true\n[server.validation]\nenabled=true\n'
        'max_body_bytes=2097152\nmax_errors=16\n')
    run('openapi-to-rust', 'generate', '--config', config, '--json')
    fragment = (server / 'src/generated/REQUIRED_DEPS.toml').read_text()
    (server / 'Cargo.toml').write_text(
        '[package]\nname="shepherd-plan-server-check"\nversion="0.0.0"\nedition="2024"\n' + fragment)
    (server / 'src/lib.rs').write_text('pub mod generated;\n')
    run('cargo', 'check', '--offline', '--manifest-path', server / 'Cargo.toml')

    frontend = scratch / 'frontend'
    frontend.mkdir()
    (frontend / 'node_modules').symlink_to(repo / 'ui/node_modules', target_is_directory=True)
    config = scratch / 'orval.config.mjs'
    config.write_text('export default ' + json.dumps({
        'client': {'input': str(spec), 'output': {
            'target': str(frontend / 'api/client.ts'),
            'schemas': str(frontend / 'api/model'),
            'client': 'react-query', 'httpClient': 'fetch', 'mode': 'tags-split',
            'packageJson': str(repo / 'ui/package.json'),
            'override': {'query': {'version': 5}, 'enumGenerationType': 'const'}}},
        'validation': {'input': str(spec), 'output': {
            'target': str(frontend / 'validation/index.ts'), 'client': 'zod', 'mode': 'tags-split'}}
    }) + ';\n')
    run(node, repo / 'ui/node_modules/orval/dist/bin/orval.mjs', '--config', config)
    (frontend / 'tsconfig.json').write_text(json.dumps({'compilerOptions': {
        'target': 'es2023', 'lib': ['ES2023', 'DOM'], 'module': 'esnext',
        'moduleResolution': 'bundler', 'strict': True, 'skipLibCheck': True,
        'noEmit': True, 'verbatimModuleSyntax': True, 'erasableSyntaxOnly': True,
        'noUnusedLocals': True, 'noUnusedParameters': True
    }, 'include': ['api', 'validation']}))
    run(repo / 'ui/node_modules/.bin/tsc', '--project', frontend / 'tsconfig.json')
    print('PASS: generated Rust models/server and React Query v5/Zod output compile.')


if __name__ == '__main__':
    main()
