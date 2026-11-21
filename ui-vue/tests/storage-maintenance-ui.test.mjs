import { locale } from "../dist/features/i18n/locale-service.js";
locale.value = "ru";
import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {
  maintenanceActions, maintenanceConfirmation, maintenanceResultLabel,
} from '../dist/features/settings/storage-maintenance-ui.js';

const panel = readFileSync(new URL('../src/features/settings/StorageManagementPanel.vue',import.meta.url),'utf8');

test('safe maintenance runs without an OS dialog; destructive actions require explicit confirmation',()=>{
  assert.equal(maintenanceActions.check.destructive,false);
  assert.equal(maintenanceActions.analyze.destructive,false);
  assert.equal(maintenanceActions.checkpoint.destructive,false);
  for(const id of ['vacuum','retention','prune']){
    assert.equal(maintenanceActions[id].destructive,true);
    assert.ok(maintenanceConfirmation(id).length>40);
  }
  assert.doesNotMatch(panel,/window\.confirm|dialog\.confirm/);
  assert.match(panel,/v-if="pendingAction"/);
  assert.match(panel,/@click="confirmPending"/);
  assert.match(panel,/@click="pendingAction=null"/);
});
test('operation results are presented next to the buttons, errors are visible',()=>{
  assert.match(panel,/v-if="maintenanceFeedback"/);
  assert.match(panel,/role="group" :aria-label="t\('storage\.confirm_maintenance'\)"/);
  assert.match(panel,/maintenanceFeedback\.type==='error'\?'alert':'status'/);
  assert.match(panel,/v-if="pinFeedback"/);
  assert.match(panel,/v-if="policyFeedback"/);
  assert.match(panel,/actionDisabled\(action\)/);
  assert.match(maintenanceResultLabel('checkpoint','WAL checkpoint выполнен',0),/WAL checkpoint выполнен/);
  assert.doesNotMatch(maintenanceResultLabel('vacuum','VACUUM завершён',0),/Удалено снимков/);
  assert.match(maintenanceResultLabel('retention','готово',3),/Удалено снимков: 3/);
});
