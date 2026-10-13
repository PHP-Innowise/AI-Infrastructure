"""Form authoring over localhost, real project paths and recoverable metadata."""
from copy import deepcopy
import base64
import json
import os
from pathlib import Path
import shutil
import tempfile
import threading
import unittest
from unittest.mock import patch

from tests import test_harness_web as http_helpers
from harness import providers, web, system_editor


class SystemEditorTests(unittest.TestCase):
    request = http_helpers.HarnessWebTests.request
    post = http_helpers.HarnessWebTests.post
    close_server = http_helpers.HarnessWebTests.close_server

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(); self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.project = self.root / 'coordination'; self.project.mkdir()
        self.orders = self.root / 'orders'; self.orders.mkdir()
        self.payments = self.project / 'payments'; self.payments.mkdir()
        fake = patch.object(providers, 'discover_providers', return_value=[])
        fake.start(); self.addCleanup(fake.stop)
        self.server = web.HarnessServer(('127.0.0.1', 0), self.root / 'state', [self.project, self.orders])
        self.closed = False
        self.thread = threading.Thread(target=self.server.serve_forever, kwargs={'poll_interval': .01}, daemon=True)
        self.thread.start(); self.addCleanup(self.close_server)
        self.token = self.server.token
        self.project_id = next(p['id'] for p in self.server.sessions.projects.values() if p['path']==str(self.project))
        self.orders_id = next(p['id'] for p in self.server.sessions.projects.values() if p['path']==str(self.orders))

    def call(self, action, body):
        status, result, _ = self.post('/api/systems/' + action, body)
        self.assertEqual(200, status, result)
        return result

    def draft(self):
        draft = self.call('editor', {'project_id': self.project_id, 'config_path': 'system.json'})
        for pid, folder, sid in ((self.orders_id, '.', 'orders'), (self.project_id, 'payments', 'payments')):
            value = self.call('service', {'project_id': pid, 'folder': folder})
            value.pop('path')
            value['passport'].update(id=sid, description='Owns ' + sid, owner='team', relationships_complete=True)
            draft['services'].append(value)
        draft['name']='Form product'
        draft['services'][0]['passport']['provides']=[{'id':'order.cancelled', 'kind':'event', 'version':'1', 'sources':[]}]
        draft['services'][1]['passport']['consumes']=[{'service':'orders', 'contract':'order.cancelled', 'version':'1'}]
        return draft

    def save(self, draft):
        preview = self.call('preview', draft)
        return self.call('apply', {'preview_id':preview['preview_id']})

    def active_session(self):
        self.server.sessions.db.execute("""INSERT INTO sessions
            (id,title,project_id,project_path,provider,mode,workflow,project_context,status,created_at,updated_at)
            VALUES ('active','Fixture',?,?,'codex','plan','native',0,'running','2026-01-01','2026-01-01')""",
            (self.project_id,str(self.project)))
        self.server.sessions.db.commit()

    def test_create_reload_edit_and_plan_from_forms(self):
        draft = self.draft()
        draft['services'][0]['passport']['capabilities']=[{'id':'cancel-order','description':'Cancel orders',
            'status':'planned','keywords':['cancel'],'sources':['spec.md']}]
        draft['services'][0]['passport']['sources']=[{'path':'spec.md','kind':'spec'}]
        (self.orders / 'spec.md').write_text('Cancellation spec')
        result=self.save(draft)
        self.assertEqual(2, len(result['catalog']['services']))
        self.assertEqual('orders', result['catalog']['relationships'][0]['provider'])
        self.assertEqual('payments', result['catalog']['relationships'][0]['consumer'])
        self.assertEqual('Form product', json.loads((self.project/'system.json').read_text())['name'])
        self.assertEqual('planned', json.loads((self.orders/'ai-service.json').read_text())['capabilities'][0]['status'])
        edited = result['editor']; self.assertNotEqual(draft['revision'], edited['revision'])
        for value in edited['services']: value.pop('path')
        edited['services'][0]['passport']['description']='Updated ownership'
        self.save(edited)
        plan_status, plan, _=self.post('/api/system-runs', {'project_id':self.project_id,'config_path':'system.json',
            'task':'Investigate cancellation','change_id':'change-form','services':['orders']})
        self.assertEqual(201, plan_status, plan)
        self.assertEqual({'orders','payments'}, set(plan['plan']['context']['services']))

    def test_stale_form_and_preview_cannot_overwrite_metadata(self):
        draft=self.draft(); saved=self.save(draft)
        stale=deepcopy(draft)
        self.assertEqual(400, self.post('/api/systems/preview', stale)[0])
        current=saved['editor']
        for value in current['services']: value.pop('path')
        current['name']='Next name'
        preview=self.call('preview',current)
        service_file=self.orders/'ai-service.json'; external=service_file.read_bytes()+b'\n'
        service_file.write_bytes(external)
        self.assertEqual(400,self.post('/api/systems/apply',{'preview_id':preview['preview_id']})[0])
        self.assertEqual(external,service_file.read_bytes())
        self.assertEqual('Form product',json.loads((self.project/'system.json').read_text())['name'])
        self.assertEqual(400,self.post('/api/systems/apply',{'preview_id':preview['preview_id']})[0])

    def test_invalid_contracts_paths_roots_and_collisions_are_read_only(self):
        draft=self.draft()
        bad=[]
        value=deepcopy(draft);value['services'][1]['passport']['consumes'][0]['contract']='missing';bad.append(value)
        value=deepcopy(draft);value['services'][1]['passport']['id']='orders';bad.append(value)
        value=deepcopy(draft);value['services'][0]['passport']['sources']=[{'path':'../secret','kind':'code'}];bad.append(value)
        value=deepcopy(draft);value['services'][0]['passport']['sources']=[{'path':'.env','kind':'code'}];bad.append(value)
        value=deepcopy(draft);value['services'][0]['project_id']='unregistered';bad.append(value)
        value=deepcopy(draft);value['services'][0]['root']=str(self.root);bad.append(value)
        for value in bad:
            self.assertEqual(400,self.post('/api/systems/preview',value)[0])
        self.assertFalse((self.project/'system.json').exists());self.assertFalse((self.orders/'ai-service.json').exists())
        (self.orders/'ai-service.json').write_text('{"application":"unrelated"}')
        self.assertEqual(400,self.post('/api/systems/service',{'project_id':self.orders_id})[0])
        self.assertEqual(400,self.post('/api/systems/preview',draft)[0])
        self.assertEqual({'application':'unrelated'},json.loads((self.orders/'ai-service.json').read_text()))

    def test_links_hardlinks_and_replaced_directories_are_refused(self):
        draft=self.draft();preview=self.call('preview',draft)
        self.orders.rename(self.root/'original-orders');self.orders.mkdir()
        self.assertEqual(400,self.post('/api/systems/apply',{'preview_id':preview['preview_id']})[0])
        self.assertFalse((self.orders/'ai-service.json').exists())
        self.orders.rmdir();self.orders.symlink_to(self.root/'original-orders',target_is_directory=True)
        self.assertEqual(400,self.post('/api/systems/service',{'project_id':self.orders_id})[0])
        self.orders.unlink();self.orders.mkdir()
        outside=self.root/'outside';outside.write_text('{}');(self.orders/'ai-service.json').symlink_to(outside)
        self.assertEqual(400,self.post('/api/systems/service',{'project_id':self.orders_id})[0])
        (self.orders/'ai-service.json').unlink();os.link(outside,self.orders/'ai-service.json')
        self.assertEqual(400,self.post('/api/systems/service',{'project_id':self.orders_id})[0])

    def test_write_failure_rolls_back_all_metadata(self):
        draft=self.draft();preview=self.call('preview',draft)
        original=system_editor.replace; calls=[]
        def fail_second(*args):
            calls.append(args)
            if len(calls)==2: raise OSError('Injected write failure')
            return original(*args)
        with patch.object(system_editor,'replace',side_effect=fail_second):
            status,result,_=self.post('/api/systems/apply',{'preview_id':preview['preview_id']})
        self.assertEqual(400,status,result);self.assertIn('restored',result['error'])
        for root in (self.orders,self.payments): self.assertFalse((root/'ai-service.json').exists())
        self.assertFalse((self.project/'system.json').exists())
        self.assertFalse(self.server.sessions.db.execute('SELECT 1 FROM system_edits').fetchone())

    def test_crash_recovery_restores_partial_writes_and_acknowledges_complete_writes(self):
        draft=self.draft();preview=self.call('preview',draft)
        files=self.server.systems.editor.previews[preview['preview_id']]['files']
        for complete in (False,True):
            with self.server.sessions.lock:
                self.server.sessions.db.execute('INSERT INTO system_edits VALUES (?,?)',('crash',json.dumps({'files':files})))
                self.server.sessions.db.commit()
                for f in (files if complete else files[:1]):
                    system_editor.replace(Path(f['root']),f['name'],base64.b64decode(f['after']),f['before'],0o600)
                self.server.systems.editor.recover()
                self.assertFalse(self.server.sessions.db.execute('SELECT 1 FROM system_edits').fetchone())
            self.assertEqual(complete,(self.project/'system.json').exists())
            self.assertEqual(complete,(self.orders/'ai-service.json').exists())

    def test_recovery_preserves_external_changes_and_blocks_overlapping_system(self):
        draft=self.draft();preview=self.call('preview',draft)
        files=self.server.systems.editor.previews[preview['preview_id']]['files']
        with self.server.sessions.lock:
            self.server.sessions.db.execute('INSERT INTO system_edits VALUES (?,?)',('crash',json.dumps({'files':files})))
            self.server.sessions.db.commit()
        (self.orders/'ai-service.json').write_text('External edit')
        self.assertEqual(400,self.post('/api/systems/editor',{'project_id':self.project_id,'config_path':'system.json'})[0])
        self.assertEqual('External edit',(self.orders/'ai-service.json').read_text())
        self.assertEqual(400,self.post('/api/systems/service',{'project_id':self.orders_id})[0])
        self.assertTrue(self.server.sessions.db.execute('SELECT 1 FROM system_edits').fetchone())

    def test_active_sessions_block_apply_and_csrf_protects_editor(self):
        draft=self.draft();preview=self.call('preview',draft)
        with self.server.sessions.lock:
            self.active_session()
        self.assertEqual(400,self.post('/api/systems/apply',{'preview_id':preview['preview_id']})[0])
        self.assertFalse((self.project/'system.json').exists())
        self.assertEqual(403,self.post('/api/systems/editor',{'project_id':self.project_id,'config_path':'system.json'},headers={'X-Harness-Token':None})[0])
        self.assertEqual(400,self.post('/api/systems/editor?unexpected=1',{'project_id':self.project_id,'config_path':'system.json'})[0])
        with self.server.sessions.lock:
            self.server.sessions.db.execute("DELETE FROM sessions WHERE id='active'");self.server.sessions.db.commit()

    def test_recovery_preserves_external_permissions(self):
        preview=self.call('preview',self.draft())
        files=self.server.systems.editor.previews[preview['preview_id']]['files']
        first=files[0]; target=Path(first['root'])/first['name']
        system_editor.replace(Path(first['root']),first['name'],base64.b64decode(first['after']),first['before'],0o600)
        target.chmod(0o644)
        with self.server.sessions.lock:
            self.server.sessions.db.execute('INSERT INTO system_edits VALUES (?,?)',('crash',json.dumps({'files':files})))
            self.server.sessions.db.commit()
            self.server.systems.editor.recover()
        self.assertEqual(base64.b64decode(first['after']),target.read_bytes())
        self.assertEqual(0o644,target.stat().st_mode & 0o777)
        self.assertTrue(self.server.sessions.db.execute('SELECT 1 FROM system_edits').fetchone())
        self.assertEqual(400,self.post('/api/systems/service',{'project_id':self.orders_id})[0])

    def test_recovery_waits_for_active_sessions_and_restores_existing_bytes_and_mode(self):
        saved=self.save(self.draft())['editor']
        for value in saved['services']: value.pop('path')
        target=self.orders/'ai-service.json';target.chmod(0o640)
        original=target.read_bytes()
        saved['name']='Changed system name'
        saved['services'][0]['passport']['description']='Changed form'
        preview=self.call('preview',saved)
        files=self.server.systems.editor.previews[preview['preview_id']]['files']
        first=files[0]
        with self.server.sessions.lock:
            self.server.sessions.db.execute('INSERT INTO system_edits VALUES (?,?)',('crash',json.dumps({'files':files})))
            self.active_session()
            system_editor.replace(Path(first['root']),first['name'],base64.b64decode(first['after']),first['before'],first['before']['mode'])
            self.server.systems.editor.recover()
            self.assertNotEqual(original,target.read_bytes())
            self.assertTrue(self.server.sessions.db.execute('SELECT 1 FROM system_edits').fetchone())
            self.server.sessions.db.execute("DELETE FROM sessions WHERE id='active'");self.server.sessions.db.commit()
            # Reconstructing the editor runs the same durable recovery as a restart.
            system_editor.SystemEditor(self.server.sessions)
        self.assertEqual(original,target.read_bytes())
        self.assertEqual(0o640,target.stat().st_mode & 0o777)
        self.assertFalse(self.server.sessions.db.execute('SELECT 1 FROM system_edits').fetchone())

    def test_nested_config_and_service_passport_paths_remain_supported(self):
        (self.project/'catalog').mkdir();(self.orders/'metadata').mkdir()
        draft=self.call('editor',{'project_id':self.project_id,'config_path':'catalog/product.json'})
        service=self.call('service',{'project_id':self.orders_id});service.pop('path')
        service['manifest']='metadata/service.json';service['passport'].update(id='orders',description='Orders',owner='team')
        draft['services']=[service]
        result=self.save(draft)
        self.assertEqual('catalog/product.json',result['editor']['config_path'])
        self.assertTrue((self.orders/'metadata/service.json').exists())

    def test_preview_storage_is_bounded_before_writes(self):
        saved=self.save(self.draft())['editor']
        for service in saved['services']: service.pop('path')
        target=self.orders/'ai-service.json'
        target.write_bytes(target.read_bytes()+b' '*9000)
        draft=self.call('editor',{'project_id':self.project_id,'config_path':'system.json'})
        for service in draft['services']: service.pop('path')
        original=target.read_bytes()
        with patch.object(system_editor,'MAX_BYTES',1024):
            status,result,_=self.post('/api/systems/preview',draft)
        self.assertEqual(400,status,result);self.assertIn('too large',result['error'])
        self.assertEqual(original,target.read_bytes())
        self.assertFalse(self.server.sessions.db.execute('SELECT 1 FROM system_edits').fetchone())

    def test_generated_service_roots_survive_workspace_relocation(self):
        self.save(self.draft())
        config=json.loads((self.project/'system.json').read_text())
        self.assertEqual(['../orders','payments'],[s['root'] for s in config['services']])
        moved=self.root/'another-checkout';moved.mkdir()
        shutil.copytree(self.project,moved/'coordination')
        shutil.copytree(self.orders,moved/'orders')
        system=system_editor.System(moved/'coordination'/'system.json',[moved])
        self.assertEqual(moved/'orders',system.services['orders']['root'])
        self.assertEqual(moved/'coordination'/'payments',system.services['payments']['root'])
        self.assertEqual('available',system.services['orders']['access'])


if __name__=='__main__': unittest.main()
