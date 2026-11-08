import ReactDOM from 'react-dom/client';
import {initializeBrowser} from './browser';
import './themes.css';
import './style.css';
async function start(){
 try{await initializeBrowser();await import('./main');}
 catch(error){ReactDOM.createRoot(document.getElementById('root')!).render(<main className="login-page"><section className="login-card"><h1>Could not open Cirra</h1><p role="alert">{(error as Error).message}</p><a className="primary-button" href="/">Return to sign in</a></section></main>);}
}
void start();
