import { Paper, PaperMetadata } from '../types';

// Simulate network delay
const delay = (ms: number) => new Promise(resolve => setTimeout(resolve, ms));

// Simulate random failures (30% failure rate on first attempt)
const shouldFail = (retryCount: number): boolean => {
  if (retryCount === 0) {
    return Math.random() < 0.3;
  }
  // Lower failure rate on retries
  return Math.random() < 0.1;
};

// Generate fake PubMed IDs
const generatePMID = (index: number): string => {
  return `${30000000 + index}`;
};

// Generate fake metadata
export const fetchMetadata = async (paperId: string, retryCount: number = 0): Promise<PaperMetadata> => {
  await delay(300 + Math.random() * 500);
  
  if (shouldFail(retryCount)) {
    throw new Error('Failed to fetch metadata');
  }

  const pmid = generatePMID(parseInt(paperId.split('-')[1] || '0'));
  
  return {
    pmid,
    title: `Study on ${['Alzheimer Disease', 'Cancer Immunotherapy', 'COVID-19 Vaccines', 'Neural Networks', 'Gene Therapy'][Math.floor(Math.random() * 5)]}: A Comprehensive Analysis`,
    authors: [
      'Smith JA',
      'Johnson BD',
      'Williams CL',
      'Brown DE'
    ].slice(0, 2 + Math.floor(Math.random() * 3)),
    journal: ['Nature', 'Science', 'Cell', 'The Lancet', 'NEJM'][Math.floor(Math.random() * 5)],
    publicationDate: `2024-${String(Math.floor(Math.random() * 12) + 1).padStart(2, '0')}-${String(Math.floor(Math.random() * 28) + 1).padStart(2, '0')}`,
    doi: `10.1000/${pmid}`
  };
};

// Generate fake abstract
export const fetchAbstract = async (paperId: string, retryCount: number = 0): Promise<string> => {
  await delay(400 + Math.random() * 600);
  
  if (shouldFail(retryCount)) {
    throw new Error('Failed to fetch abstract');
  }

  return `Background: This study investigates the mechanisms underlying cellular responses to external stimuli. 
Methods: We conducted a randomized controlled trial with ${100 + Math.floor(Math.random() * 400)} participants over a ${6 + Math.floor(Math.random() * 18)}-month period. 
Results: Our findings demonstrate significant improvements in primary outcomes (p < 0.001), with effect sizes ranging from 0.${Math.floor(Math.random() * 9)} to 0.${Math.floor(Math.random() * 9)}. 
Conclusions: These results have important implications for clinical practice and suggest new avenues for future research.`;
};

// Generate fake PDF URL
export const fetchPDF = async (paperId: string, retryCount: number = 0): Promise<string> => {
  await delay(500 + Math.random() * 800);
  
  if (shouldFail(retryCount)) {
    throw new Error('Failed to fetch PDF');
  }

  const pmid = generatePMID(parseInt(paperId.split('-')[1] || '0'));
  return `https://www.ncbi.nlm.nih.gov/pmc/articles/PMC${pmid}/pdf/`;
};

// Main search function that returns paper IDs
export const searchPapers = async (query: string): Promise<string[]> => {
  await delay(500);
  
  // Return 5-10 fake paper IDs
  const count = 5 + Math.floor(Math.random() * 6);
  return Array.from({ length: count }, (_, i) => `paper-${i}`);
};
