import { locale } from "../dist/features/i18n/locale-service.js";
locale.value = "ru";
import assert from 'node:assert/strict';
import test from 'node:test';
import { createShellState, enterShellWorkspace, leaveShellWorkspace } from '../dist/core/shell-state.js';
import { createConnectionProjectsController, createConnectionProjectsState, parseConnectionProjects, validateProjectName } from '../dist/features/connections/projects-controller.js';

function setup(invoke) {
  const shell = createShellState(); enterShellWorkspace(shell, 'admin');
  const state = createConnectionProjectsState(); const calls=[];
  const controller = createConnectionProjectsController(shell, state, async (cmd,args) => {
    calls.push([cmd,args]); return invoke(cmd,args);
  }, async () => true);
  return {shell,state,calls,controller};
}
test('projects validate and discard untrusted fields', () => {
  assert.deepEqual(parseConnectionProjects([{id:4,name:'pgXray',password:'secret'}]),[{id:4,name:'pgXray'}]);
  assert.match(validateProjectName('\n'), /Название/);
  assert.equal(validateProjectName('dba-trainer'),null);
  assert.throws(()=>parseConnectionProjects([{id:'4',name:'x'}]),/Некорректный/);
});
test('create, rename and delete preserve distinct IPC operations',async()=>{
  let rows=[];
  const {controller,calls,state}=setup(async (cmd,args)=>{
    if(cmd==='list_connection_projects')return rows;
    if(cmd==='create_connection_project')rows=[{id:1,name:args.name}];
    if(cmd==='rename_connection_project')rows=[{id:1,name:args.name}];
    if(cmd==='delete_connection_project')rows=[];
    return null;
  });
  assert.equal(await controller.create('  dba-trainer '),true);
  assert.equal(state.items[0].name,'dba-trainer');
  assert.equal(await controller.rename(1,'pgXray'),true);
  assert.equal(await controller.remove(1),true);
  assert.deepEqual(state.items,[]);
  assert.deepEqual(calls.filter(c=>!c[0].startsWith('list_')),[
    ['create_connection_project',{name:'dba-trainer'}],
    ['rename_connection_project',{id:1,name:'pgXray'}],
    ['delete_connection_project',{id:1}],
  ]);
});
test('connection move only changes project_id',async()=>{
  const {controller,calls}=setup(async cmd=>cmd==='list_connection_projects'?[]:null);
  assert.equal(await controller.move(42,3),true);
  assert.deepEqual(calls[0],['assign_connection_project',{connectionId:42,projectId:3}]);
  assert.equal(await controller.move(42,null),true);
});
test('a response after logout does not recreate projects',async()=>{
  let resolve;
  const slow=new Promise(r=>resolve=r);
  const {shell,controller,state}=setup(async cmd=>cmd==='list_connection_projects'?slow:null);
  const pending=controller.refresh();
  leaveShellWorkspace(shell); controller.reset();
  resolve([{id:1,name:'Old user'}]);
  assert.equal(await pending,false);
  assert.deepEqual(state.items,[]);
});
