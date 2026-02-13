export interface PaperMetadata {
  pmid: string;
  title: string;
  authors: string[];
  journal: string;
  publicationDate: string;
  doi?: string;
}

export interface Paper {
  id: string;
  pmid: string;
  metadata: PaperMetadata;
  abstract: string;
  pdfUrl: string;
}

export enum FetchStatus {
  PENDING = 'pending',
  FETCHING = 'fetching',
  SUCCESS = 'success',
  FAILED = 'failed',
  RETRYING = 'retrying'
}

export interface PaperComponent {
  name: 'metadata' | 'abstract' | 'pdf';
  status: FetchStatus;
  retryCount: number;
  error?: string;
}

export interface PaperFetchState {
  paper: Partial<Paper>;
  components: {
    metadata: PaperComponent;
    abstract: PaperComponent;
    pdf: PaperComponent;
  };
  overallStatus: FetchStatus;
}

export interface RetryQueueItem {
  paperId: string;
  componentName: 'metadata' | 'abstract' | 'pdf';
  retryCount: number;
}
