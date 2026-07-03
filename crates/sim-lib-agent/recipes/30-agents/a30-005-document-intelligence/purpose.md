# Document Intelligence Card

This recipe records deterministic document extraction over a synthetic invoice.
The setup quotes fake-vision OCR output, schema field mapping, confidence
thresholding, and a validation Card that accepts the mapped fields.

The fixture uses only local synthetic inputs. It gives recipe browsers a stable
document-intelligence trace without live OCR, network access, or external files.
