import React from 'react';
import ReactDOM from 'react-dom/client';
import Login from './Login';
import { initializeTheme } from './theme';
import './themes.css';
import './style.css';
import './reactions.css';

initializeTheme();

ReactDOM.createRoot(document.getElementById('root')!).render(<React.StrictMode><Login /></React.StrictMode>);
