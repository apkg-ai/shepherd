#!/usr/bin/env python3
"""Exercise the future v1 API with four existing credentials; creates sample data.

Run only against a disposable v1 daemon. This script was syntax checked during
planning, not executed against the MVP. Tokens are read from protected files.
"""
import argparse
import json
from pathlib import Path
from urllib.error import HTTPError
from urllib.parse import urlencode, urlparse
from urllib.request import Request, build_opener, ProxyHandler, HTTPRedirectHandler
from uuid import uuid4


class NoRedirect(HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--url', default='http://127.0.0.1:7437')
    for name in ('owner', 'planner', 'reviewer', 'executor'):
        parser.add_argument('--' + name + '-token-file', required=True)
    args = parser.parse_args()
    parsed = urlparse(args.url)
    if parsed.scheme != 'http' or parsed.hostname not in ('127.0.0.1', 'localhost'):
        parser.error('Use an HTTP loopback daemon.')
    tokens = {}
    for name in ('owner', 'planner', 'reviewer', 'executor'):
        path = Path(getattr(args, name + '_token_file'))
        if path.stat().st_mode & 0o077:
            parser.error(f'{name} token file must be private (0600).')
        tokens[name] = path.read_text().strip()
    opener = build_opener(ProxyHandler({}), NoRedirect())

    def call(actor, method, path, body=None, revision=None, lease=None, key=None,
             expected=200):
        headers = {'Authorization': 'Bearer ' + tokens[actor]}
        if method != 'GET':
            headers['Idempotency-Key'] = key or str(uuid4())
        if revision is not None:
            headers['If-Match'] = f'"{revision}"'
        if lease:
            headers['X-Lease-Token'] = lease
        data = None
        if body is not None:
            headers['Content-Type'] = 'application/json'
            data = json.dumps(body).encode()
        req = Request(args.url.rstrip('/') + path, data=data, headers=headers, method=method)
        try:
            response = opener.open(req, timeout=15)
        except HTTPError as exc:
            response = exc
        with response:
            payload = json.loads(response.read())
            assert response.status == expected, (method, path, response.status, payload)
            return payload

    principals = {a: call(a, 'GET', '/api/v1/principal') for a in tokens}
    assert principals['owner']['kind'] == 'human'
    assert len({p['id'] for p in principals.values()}) == 4
    project = call('owner', 'POST', '/api/v1/projects', {
        'name': 'Space Game acceptance ' + str(uuid4())[:8],
        'settings': {'proposal_gate': True, 'planning_required': False,
                     'plan_review': 'human', 'work_review': 'human'}}, expected=201)
    root = '/api/v1/projects/' + project['id']
    goal = call('owner', 'POST', root + '/goals', {'title': 'Combat prototype'}, expected=201)
    call('owner', 'POST', root + '/goals', {'title': 'Independent colony prototype'}, expected=201)
    foundation = call('owner', 'POST', root + '/epics', {
        'goal_id': goal['id'], 'title': 'Combat foundation'}, expected=201)
    targeting = call('owner', 'POST', root + '/epics', {
        'goal_id': goal['id'], 'title': 'Targeting'}, expected=201)
    call('owner', 'POST', root + '/dependencies', {'level': 'epic',
         'dependent_id': targeting['id'], 'prerequisite_id': foundation['id']}, expected=201)
    prerequisite = call('owner', 'POST', root + '/tasks', {
        'epic_id': foundation['id'], 'title': 'Define combat fixtures', 'type_key': 'research',
        'work_review': 'none'}, expected=201)
    # Owner sets explicit policies; proposal acceptance is covered separately by A03/A06.
    task = call('owner', 'POST', root + '/tasks', {
        'epic_id': targeting['id'], 'title': 'Implement targeting', 'type_key': 'code',
        'planning_required': True, 'plan_review': 'agent', 'work_review': 'human'}, expected=201)

    def current(task_id):
        return call('owner', 'GET', root + '/tasks/' + task_id)

    def claim(actor, target, phase, submission=None):
        body = {'phase': phase, 'ttl_seconds': 300}
        if submission:
            body['submission_id'] = submission
        return call(actor, 'POST', root + '/tasks/' + target['id'] + '/claims', body,
                    revision=current(target['id'])['revision'], expected=201)

    def document(actor, target, kind, body):
        return call(actor, 'POST', root + '/documents', {
            'owner_kind': 'task', 'owner_id': target['id'], 'kind': kind,
            'title': kind.capitalize() + ' evidence', 'body': body, 'links': []}, expected=201)

    def report(actor, grant, revision_id):
        body = {'outcome': 'succeeded', 'summary': 'Acceptance fixture work recorded.',
                'document_revision_ids': [revision_id], 'links': []}
        path = root + '/claims/' + grant['claim']['id'] + '/report'
        key = str(uuid4())
        result = call(actor, 'POST', path, body, lease=grant['lease_token'], key=key, expected=201)
        replay = call(actor, 'POST', path, body, lease=grant['lease_token'], key=key, expected=201)
        assert replay['id'] == result['id']

    def pending(target, kind):
        query = urlencode({'task_id': target['id'], 'status': 'pending', 'kind': kind})
        items = call('owner', 'GET', root + '/submissions?' + query)['items']
        assert len(items) == 1
        return items[0]

    plan_claim = claim('planner', task, 'plan')
    plan_doc = document('planner', task, 'plan',
                        'Select nearest valid target. Test empty candidates and deterministic ties.')
    plan_id = plan_doc['latest_revision_id']
    call('planner', 'POST', root + '/tasks/' + task['id'] + '/plan',
         {'document_revision_id': plan_id}, revision=current(task['id'])['revision'])
    report('planner', plan_claim, plan_id)
    submission = pending(task, 'plan')
    call('planner', 'POST', root + '/tasks/' + task['id'] + '/claims',
         {'phase': 'review', 'submission_id': submission['id']},
         revision=current(task['id'])['revision'], expected=403)
    review_claim = claim('reviewer', task, 'review', submission['id'])
    call('reviewer', 'POST', root + '/submissions/' + submission['id'] + '/reviews',
         {'decision': 'approve', 'reason': 'Fixture covers empty and tied candidates.',
          'claim_id': review_claim['claim']['id']}, revision=submission['revision'],
         lease=review_claim['lease_token'], expected=201)
    assert not current(task['id'])['eligibility']['can_execute']
    grant = claim('executor', prerequisite, 'execute')
    output = document('executor', prerequisite, 'finding', 'Sample combat fixtures defined.')
    report('executor', grant, output['latest_revision_id'])
    assert call('owner', 'GET', root + '/epics/' + foundation['id'])['status'] == 'done'
    assert current(task['id'])['eligibility']['can_execute']
    execution = claim('executor', task, 'execute')
    assert execution['context']['selected_plan'][0]['id'] == plan_id
    result = document('executor', task, 'handoff',
                      'Synthetic acceptance output; this script does not implement game code.')
    report('executor', execution, result['latest_revision_id'])
    work = pending(task, 'work')
    call('executor', 'POST', root + '/submissions/' + work['id'] + '/reviews',
         {'decision': 'approve', 'reason': 'Attempted self approval'},
         revision=work['revision'], expected=403)
    call('owner', 'POST', root + '/submissions/' + work['id'] + '/reviews',
         {'decision': 'approve', 'reason': 'Synthetic handoff accepted.'},
         revision=work['revision'], expected=201)
    assert current(task['id'])['status'] == 'done'
    assert call('owner', 'GET', root + '/epics/' + targeting['id'])['status'] == 'done'
    print(json.dumps({'project_id': project['id'], 'task_id': task['id'],
                      'result': 'planning, independent review, unlock, handoff, retry passed'}))


if __name__ == '__main__':
    main()
