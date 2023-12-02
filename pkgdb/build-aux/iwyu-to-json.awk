BEGIN {
  count = 0;
  curr = "SKIP";
  target = "NULL";
  delete adds[0];
  addsIdx = 0;
  delete removes[0];
  removesIdx = 0;
  delete forwards[0];
  forwardsIdx = 0;
  printf( "[\n" );
}


/^\(.* has correct .*\)$/ {
  curr = "SKIP";
  addsIdx = 0;
  forwardsIdx = 0;
  removesIdx = 0;

  if ( target ~ /\/nix\/store/ )
    {
      target = "NULL";
      next;
    }

  if ( 0 == count )
    {
      printf( "  " );
    }
  else
    {
      printf( ", " );
    }
  count++;
  target = gensub( /\(/, "", "g", $1 );
  printf(                                                                      \
    "{ \"file\": \"%s\", \"adds\": [], \"removes\": [], \"forwards\": [] }\n"  \
  , gensub( /^.*\/include\//, "", 1, target )                                  \
  );

  target = "NULL";
  next;
}


/^---$/ {
  curr = "SKIP";
  target = "NULL";
  addsIdx = 0;
  removesIdx = 0;
  forwardsIdx = 0;
  next;
}

/^$/ {
  next;
}


/^.* should add these lines:$/ {
  curr = "add";
  addsIdx = 0;
  forwardsIdx = 0;
  target = $1;
  next;
}


/^.* should remove these lines:$/ {
  curr = "remove";
  removesIdx = 0;
  next;
}


/^The full include-list for [^[:space:]]*:$/ {
  if ( target ~ /\/nix\/store/ )
    {
      curr = "SKIP";
      target = "NULL";
      addsIdx = 0;
      forwardsIdx = 0;
      removesIdx = 0;
      next;
    }

  if ( 0 == count )
    {
      printf( "  " );
    }
  else
    {
      printf( ", " );
    }
  count++;

  printf( "{ \"file\": \"%s\", \"adds\": ["          \
        , gensub( /^.*\/include\//, "", 1, target )  \
        );

  if ( 0 < addsIdx )
    {
      printf( "\"%s\"", adds[1] );
      for ( i = 2; i <= addsIdx; i++ )
        {
          printf( ", \"%s\"", adds[i] );
        }
    }

  printf( "], \"removes\": [" );
  if ( 0 < removesIdx )
    {
      printf( "\"%s\"", removes[1] );
      for ( i = 2; i <= removesIdx; i++ )
        {
          printf( ", \"%s\"", removes[i] );
        }
    }

  printf( "], \"forwards\": [" );
  if ( 0 < forwardsIdx )
    {
      printf( "\"%s\"", forwards[1] );
      for ( i = 2; i <= forwardsIdx; i++ )
        {
          printf( ", \"%s\"", forwards[i] );
        }
    }

  printf( "] }\n" );

  curr = "new";
  target = "NULL";
  addsIdx = 0;
  forwardsIdx = 0;
  removesIdx = 0;
  next;
}


/^#include [^[:space:]]+$/ {
  if ( curr == "add" )
    {
      addsIdx++;
      adds[addsIdx] = gensub( /"/, "\\\\\"", "g", $2 );
    }
  next;
}


/^namespace [^[:space:]]+$/ {
  if ( curr == "add" )
    {
      forwardsIdx++;
      forwards[forwardsIdx] = $0;
    }
  next;
}


/^- #include [^[:space:]]+$/ {
  if ( curr == "remove" )
    {
      removesIdx++;
      removes[removesIdx] = gensub( /"/, "\\\\\"", "g", $3 );
    }
  next;
}


END {
  printf( "]\n" );
}
