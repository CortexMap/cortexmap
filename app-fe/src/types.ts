export interface BrainRegionLocation {
  hemisphere: string;
  lobe: string;
  anatomical_region: string;
}

export interface FunctionDiseases {
  function_description: string;
  disease_description: string;
}

export interface BrainRegion {
  id: string;
  name: string;
  location: BrainRegionLocation;
  function_diseases: FunctionDiseases;
}
