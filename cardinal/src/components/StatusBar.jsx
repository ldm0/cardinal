import React from 'react';
import './StatusBar.css';

const StatusBar = ({ scannedFiles, processedEvents, isReady }) => {
  return (
    <div className='status-bar'>
      <div className='status-section'>
        <span className={`readiness-indicator ${isReady ? 'ready' : 'not-ready'}`}>{isReady ? '●' : '○'}</span>
        <span className='status-text'>{isReady ? 'Ready' : 'Initializing'}</span>
      </div>
      <div className='status-section'>
        <span className='status-label'>Files:</span>
        <span className='status-value'>{scannedFiles.toLocaleString()}</span>
      </div>
      <div className='status-section'>
        <span className='status-label'>Events:</span>
        <span className='status-value'>{processedEvents.toLocaleString()}</span>
      </div>
    </div>
  );
};

export default StatusBar;
