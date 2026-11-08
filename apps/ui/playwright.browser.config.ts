import {defineConfig} from '@playwright/test';
export default defineConfig({
 testDir:'./tests',testMatch:['browser-client.spec.ts'],outputDir:'./test-results-browser',workers:1,timeout:90000,
 use:{baseURL:'http://127.0.0.1:3111',trace:'retain-on-failure'},
 webServer:[
  {command:'cargo run --manifest-path ../../Cargo.toml -p cirra-device --example browser_fixture --locked',url:'http://127.0.0.1:3112/healthz',timeout:180000,reuseExistingServer:false},
  {command:'npx vite --config vite.browser-test.config.ts',url:'http://127.0.0.1:3111',env:{VITE_CIRRA_BROWSER:'true',VITE_CIRRA_RELAY:'http://127.0.0.1:3111'},timeout:60000,reuseExistingServer:false}
 ]
});
