const referenceInput=JSON.parse(readFile(arguments[0]));
print(JSON.stringify(ChessVisionCore.evidence(referenceInput.squares,referenceInput.occupied,referenceInput.pieces,referenceInput.coverage)));
