// Narrow behavioral oracle for the disposable baseline fixture, NOT a browser.
// Runs generated inline JS with DOM stubs. CSS validity/appearance remain unverified.
const fs = require('fs');
const vm = require('vm');
const html = fs.readFileSync(process.argv[2], 'utf8');
const ids = {};
for (const tag of html.matchAll(/<(input|button)\b([^>]+)>/g)) {
  const attrs = Object.fromEntries([...tag[2].matchAll(/([\w-]+)="([^"]*)"/g)].map(x => [x[1],x[2]]));
  if (attrs.id) ids[attrs.id] = {...attrs, listeners:{}, addEventListener(type, fn) {this.listeners[type]=fn;}};
}
const classes = new Set();
const body = {style:{},classList:{toggle(name){classes.has(name)?classes.delete(name):classes.add(name);}}};
const document = {body,getElementById(id){if(!ids[id])throw Error('unsupported element '+id);return ids[id];}};
const result = {kind:'DOM-stub behavior only; no browser CSS parsing/rendering',samples:[],errors:[],hueCheck:'unavailable'};
try {
  const context=vm.createContext({document});
  for (const match of html.matchAll(/<script[^>]*>([\s\S]*?)<\/script>/g)) vm.runInContext(match[1],context,{timeout:1000});
  const slider=Object.values(ids).find(x=>x.type==='range');
  if(!slider || !slider.listeners.input)throw Error('missing fixture slider handler');
  for(const fraction of [0,1/3,2/3,1]) {
    slider.value=String(Number(slider.min)+(Number(slider.max)-Number(slider.min))*fraction);
    context.event={target:slider};context.handler=slider.listeners.input;
    vm.runInContext('handler(event)',context,{timeout:1000});
    result.samples.push({fraction,background:body.style.background});
  }
  const hues=result.samples.slice(0,3).map(x=>/hsl\(\s*([\d.-]+)/.exec(x.background||''));
  if(hues.every(Boolean))result.hueCheck=hues.every((x,i)=>Math.abs(Number(x[1])-[0,120,240][i])<=15)?'pass':'fail';
  else if(result.samples.every(x=>/linear-gradient\([\d.-]+deg,/.test(x.background||'')) && new Set(result.samples.map(x=>x.background.replace(/linear-gradient\([\d.-]+deg,/,''))).size===1)result.hueCheck='fail: fixed colors, angle only';
} catch(e) {result.errors.push(String(e));}
console.log(JSON.stringify(result,null,2));
