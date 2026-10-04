var ChessLibrary = (() => {
  const symbols={P:"♙",N:"♘",B:"♗",R:"♖",Q:"♕",K:"♔",p:"♟",n:"♞",b:"♝",r:"♜",q:"♛",k:"♚"};
  function board(fen){const ranks=fen.split(" ")[0].split("/");if(ranks.length!==8)throw Error("Invalid FEN");const squares=[];ranks.forEach((rank,r)=>{let c=0;for(const char of rank){if(/[1-8]/.test(char)){for(let i=0;i<Number(char);i++)squares.push({name:`${"abcdefgh"[c++]}${8-r}`,symbol:""});}else if(symbols[char])squares.push({name:`${"abcdefgh"[c++]}${8-r}`,symbol:symbols[char]});else throw Error("Invalid piece");}if(c!==8)throw Error("Invalid FEN rank");});return squares;}
  function interval(timing){const b=timing?.elapsed_since_previous;return b?`${(b.earliest/1e6).toFixed(2)}–${(b.latest/1e6).toFixed(2)} s`:"Unknown";}
  function matches(game,query,status){return (!status||game.status===status)&&[game.white,game.black,new Date(game.created_utc_us/1000).toISOString(),game.id].join(" ").toLowerCase().includes(query.toLowerCase());}
  return {board,interval,matches};
})();
